mod audio_proc;
mod break_in;
mod chat_history;
mod conversation;
mod piper_tts;

use audio_proc::{apply_high_pass, calculate_rms, calculate_zcr, sanitize_frame};
use conversation::ConversationEngine;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use earshot::Detector;
use log::{Level, debug, error, info, log_enabled};
use std::{collections::VecDeque, env, path::PathBuf, sync::Arc};
use tokio::sync::mpsc;
use whisper_rs::WhisperContext;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::from_filename("vadinator.env").ok();
    env_logger::init();

    // Load speech audio engine
    let ae = Arc::new(piper_tts::AudioEngine::new());

    // Load Whisper and break-in monitoring threads
    let base_path = PathBuf::from("./models");
    let model_file = env::var("WHISPER_MODEL")
        .expect("Missing: WHISPER_MODEL not set in your vadinator.env file.");
    let model_path = base_path.join(model_file);

    let ctx = WhisperContext::new_with_params(model_path, Default::default()).unwrap();
    let shared_ctx = Arc::new(ctx);
    let tx_break_in = break_in::start_break_in_worker(shared_ctx.clone(), ae.clone());

    // Load conversation engine
    let system_prompt = "You are a friendly and knowledgeable collaborator. Your tone is conversational, warm, and professional but relaxed. Avoid corporate jargon or overly formal 'As an AI' hedging. Speak like a smart friend—use natural transitions, show curiosity about the user's goals, and vary your sentence structure to keep the rhythm of the conversation lively. If the user is excited, mirror that energy; if they are frustrated, be empathetic and grounded. Keep responses punchy and avoid dry, list-heavy walls of text unless specifically asked.";
    let ce = ConversationEngine::new(shared_ctx.clone(), ae.clone(), system_prompt);

    // Say hello
    ae.tx.send("Hello, I'm ready to chat.".to_string()).ok();

    // Initialize the VAD "Detector"
    // Earshot is stateful, so it remembers the "noise" in your room.
    let mut detector = Detector::default();

    // Set up the Microphone (CPAL)
    let host = cpal::default_host();
    let device = host.default_input_device().expect("No mic found, boss.");
    // Search for a 48kHz config
    let supported_config = device
        .supported_input_configs()?
        .find(|conf| {
            // 1. Get the sample rate as a u32
            let min_rate: u32 = conf.min_sample_rate();
            let max_rate: u32 = conf.max_sample_rate();

            // 2. Check for 48kHz and Floating Point (F32)
            // If the compiler thinks sample_format() is a u32, we use a numeric match
            let is_f32 = match conf.sample_format() {
                cpal::SampleFormat::F32 => true,
                _ => false,
            };

            min_rate <= 48000 && max_rate >= 48000 && is_f32
        })
        .expect("Mic doesn't support 48kHz.")
        .with_sample_rate(48000);

    //let config = device.default_input_config()?;
    let config: cpal::StreamConfig = supported_config.into();
    let sample_rate = config.sample_rate;
    info!("Listening at {}Hz...", sample_rate);

    // Channel to send audio from the hardware thread to our logic thread
    let (tx_hw, mut rx_hw) = mpsc::channel::<Vec<f32>>(100);

    let stream = device.build_input_stream(
        &config.into(),
        move |data: &[f32], _: &_| {
            let _ = tx_hw.blocking_send(data.to_vec());
        },
        |err| error!("Mic error: {}", err),
        None,
    )?;

    stream.play()?;

    const PRE_ROLL_SIZE: usize = 32000;

    let mut recording_buffer: Vec<f32> = Vec::new();
    let mut is_recording = false;
    let mut silence_frames = 0;
    let mut break_in_counter = 0;
    let mut audio_buffer = Vec::new();
    let mut pre_roll = VecDeque::with_capacity(PRE_ROLL_SIZE);
    let mut high_pass_state = 0.0;
    let mut frame_count = 0;

    // 1.0 seconds of silence = (16000 samples / 256 samples per frame) ≈ 62 frames
    let max_silence_frames: usize = env::var("MAX_SILENCE_FRAMES")
        .ok()
        .and_then(|val| val.parse().ok())
        .unwrap_or(62); // Default 
    let min_score_recording: f32 = env::var("MIN_SCORE_RECORDING")
        .ok()
        .and_then(|val| val.parse().ok())
        .unwrap_or(0.5); // Default
    let min_score_wake: f32 = env::var("MIN_SCORE_WAKE")
        .ok()
        .and_then(|val| val.parse().ok())
        .unwrap_or(0.8); // Default

    while let Some(chunk) = rx_hw.recv().await {
        // Decimate: 48,000 / 3 = 16,000
        let decimated: Vec<f32> = chunk.iter().step_by(3).cloned().collect();
        audio_buffer.extend(decimated);

        // Earshot strictly requires exactly 256 samples
        while audio_buffer.len() >= 256 {
            // Drain the first 256 samples
            let mut frame_f32: Vec<f32> = audio_buffer.drain(..256).collect();

            // ALWAYS add the current frame to pre-roll
            pre_roll.extend(frame_f32.iter().cloned());
            // If pre-roll is too long, drop the oldest samples
            while pre_roll.len() > PRE_ROLL_SIZE {
                pre_roll.pop_front();
            }

            sanitize_frame(&mut frame_f32);
            apply_high_pass(&mut frame_f32, &mut high_pass_state, 0.95);

            let score = detector.predict_f32(&frame_f32);

            if log_enabled!(Level::Debug) {
                // Print peaks every 4 frames
                frame_count += 1;
                if frame_count & 3 == 0 {
                    let peak = frame_f32.iter().map(|s| s.abs()).fold(0.0, f32::max);
                    let bar_len = (peak * 50.0) as usize; // Map 0.0-1.0 to 0-50 chars
                    let bar = "█".repeat(bar_len.min(50));

                    let threshold = if is_recording {
                        min_score_recording
                    } else {
                        min_score_wake
                    };
                    print!("\rVol: [{:<50}] Score: {:.2}, {:.2}", bar, score, threshold);
                    use std::io::{self, Write};
                    io::stdout().flush().unwrap();

                    frame_count = 0;
                }
            }

            if ae.is_active() {
                break_in_counter += 1;
                if break_in_counter >= 31 && score > 0.8 {
                    let break_in_audio = std::mem::take(&mut pre_roll);
                    tx_break_in.send(break_in_audio.into()).ok();

                    break_in_counter = 0;
                }
                continue;
            }

            if !is_recording {
                let rms = calculate_rms(&frame_f32);
                let zcr = calculate_zcr(&frame_f32);

                if score > min_score_wake && rms > 0.00 && zcr < 60 {
                    debug!("🔥 VOICE DETECTED! (Score: {:.2})", score);
                    debug!("🎤 Starting new recording...");
                    is_recording = true;

                    // Drain last .5 second of audio as pre-fill
                    let start_idx = pre_roll.len().saturating_sub(8000);
                    recording_buffer.extend(pre_roll.drain(start_idx..));
                }
            } else {
                recording_buffer.extend_from_slice(&frame_f32);

                if score > min_score_recording {
                    // Reset the "Hang Time" timer
                    silence_frames = 0;
                } else {
                    silence_frames += 1;
                }

                // After 1 sec of silence or up to 25 secs of talking
                if silence_frames >= max_silence_frames || recording_buffer.len() > 400000 {
                    debug!(
                        "✅ Phrase complete. Total samples: {}",
                        recording_buffer.len()
                    );

                    // Send to whisper at least 1 second of audio
                    if recording_buffer.len() > 16000 {
                        let audio_to_process = std::mem::take(&mut recording_buffer);
                        ae.tx.send("Interesting. Please wait.".to_string()).ok();
                        ce.tx.send(audio_to_process).ok();
                    }

                    // Reset everything for the next phrase
                    recording_buffer.clear();
                    is_recording = false;
                    silence_frames = 0;
                }
            }
        }
    }

    Ok(())
}
