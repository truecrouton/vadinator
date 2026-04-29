mod piper_tts;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use earshot::Detector;
use piper_tts::start_speech_worker;
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc as std_mpsc,
    },
};
use tokio::sync::mpsc;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext};

/// Calculates the Zero Crossing Rate.
/// High ZCR (>60) usually indicates white noise, clicks, or "pops".
/// In a 256-sample frame (16ms) @ 16kHz:
/// - Speech usually has 10–40 crossings.
/// - Static/Pops/Clicks usually have >60 crossings.
fn calculate_zcr(frame: &[f32]) -> usize {
    frame
        .windows(2)
        .filter(|win| (win[0] >= 0.0 && win[1] < 0.0) || (win[0] < 0.0 && win[1] >= 0.0))
        .count()
}

/// Calculates the Root Mean Square (Volume).
/// Use this as a "Noise Gate" to ignore the floor hum.
fn calculate_rms(frame: &[f32]) -> f32 {
    let sq_sum: f32 = frame.iter().map(|&s| s * s).sum();
    (sq_sum / frame.len() as f32).sqrt()
}

/// High-pass filter
/// Higher Alpha (0.99): More low-end is kept, but it's more likely to overshoot
/// Lower Alpha (0.80): More low-end is cut (thinner sound), but it's much more stable.
fn apply_high_pass(frame: &mut [f32], state: &mut f32, alpha: f32) {
    for sample in frame.iter_mut() {
        let current = *sample;

        // High-pass math: y[n] = x[n] - x[n-1] + alpha * y[n-1]
        let filtered = current - *state;
        *state = current;

        // Clamp the result to keep things from panicking
        *sample = (filtered * alpha).clamp(-1.0, 1.0);
    }
}

/// Clamps and sanitizes audio to prevent NN panics.
/// Sanitization: Ensure NO values exceed 1.0 or -1.0
/// and handle any weird NaN/Inf values from the driver.
fn sanitize_frame(frame: &mut [f32]) {
    for sample in frame.iter_mut() {
        // 1. Handle NaNs (sometimes happens on mic disconnect)
        if sample.is_nan() {
            *sample = 0.0;
        }

        // 2. Clamp strictly to [-1.0, 1.0]
        *sample = sample.clamp(-1.0, 1.0);
    }
}

async fn send_to_llama(text: String) -> Result<(), reqwest::Error> {
    let client = reqwest::Client::new();

    // llama.cpp server usually runs on 8080, Ollama on 11434
    let url = "http://localhost:8080/completion";

    let body = json!({
        "prompt": format!("\n\n### User: {}\n### Assistant:", text),
        "n_predict": 128,
        "stream": false
    });

    let res = client.post(url).json(&body).send().await?;

    if res.status().is_success() {
        let response_data: serde_json::Value = res.json().await?;

        // Extract the actual string
        if let Some(llama_text) = response_data["content"].as_str() {
            // Send text to Piper here
            println!("🦙 Llama says: {}", llama_text);
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create a whisper channel
    let (tx_whisper, rx_whisper) = std_mpsc::channel::<Vec<f32>>();
    let (tx_speaker, rx_speaker) = std_mpsc::channel::<String>();
    let is_speaking = Arc::new(AtomicBool::new(false));

    start_speech_worker(rx_speaker, Arc::clone(&is_speaking));

    tx_speaker
        .send("Hello, I'm ready to chat.".to_string())
        .ok();

    // Run whisper in a thread so we don't block the audio loop
    std::thread::spawn(move || {
        // Load Whisper inside the thread
        let ctx = WhisperContext::new_with_params(
            "../models/whisper-ggml-small.en.bin",
            Default::default(),
        )
        .unwrap();
        let mut state = ctx.create_state().unwrap();

        // The thread sits here and waits for audio data
        while let Ok(audio_data) = rx_whisper.recv() {
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

            // Disable the "Standard" Whisper chatter
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            params.set_suppress_blank(true);

            state
                .full(params, &audio_data[..])
                .expect("Transcription failed");

            let mut transcript = String::new();
            for segment in state.as_iter() {
                if let Ok(text) = segment.to_str() {
                    transcript.push(' ');
                    transcript.push_str(text.trim());
                }
            }
            println!("\n[Whisper]: {}", transcript);
            tx_speaker.send(transcript.clone()).ok();

            // Fire and forget to the Llama server
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(send_to_llama(transcript));
            match result {
                Ok(()) => println!("Transcription sent."),
                Err(e) => {
                    eprintln!("Something went wrong: {:?}", e);
                }
            }
        }
    });

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
    println!("Listening at {}Hz...", sample_rate);

    // Channel to send audio from the hardware thread to our logic thread
    let (tx_hw, mut rx_hw) = mpsc::channel::<Vec<f32>>(100);

    let stream = device.build_input_stream(
        &config.into(),
        move |data: &[f32], _: &_| {
            let _ = tx_hw.blocking_send(data.to_vec());
        },
        |err| eprintln!("Mic error: {}", err),
        None,
    )?;

    stream.play()?;

    let mut recording_buffer: Vec<f32> = Vec::new();
    let mut is_recording = false;
    let mut silence_frames = 0;
    let mut audio_buffer = Vec::new();
    let mut pre_roll = VecDeque::with_capacity(8000);
    let mut high_pass_state = 0.0;
    let mut frame_count = 0;

    // 1.0 seconds of silence = (16000 samples / 256 samples per frame) ≈ 62 frames
    const MAX_SILENCE_FRAMES: usize = 62;
    const MIN_SCORE_RECORDING: f32 = 0.5;
    const MIN_SCORE_WAKE: f32 = 0.8;

    while let Some(chunk) = rx_hw.recv().await {
        // If the bot is talking, just throw this audio in the trash
        if is_speaking.load(Ordering::SeqCst) {
            continue;
        }

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
            while pre_roll.len() > 8000 {
                pre_roll.pop_front();
            }

            sanitize_frame(&mut frame_f32);
            apply_high_pass(&mut frame_f32, &mut high_pass_state, 0.95);

            let score = detector.predict_f32(&frame_f32);

            // Print peaks every 4 frames
            frame_count += 1;
            if frame_count & 3 == 0 {
                let peak = frame_f32.iter().map(|s| s.abs()).fold(0.0, f32::max);
                let bar_len = (peak * 50.0) as usize; // Map 0.0-1.0 to 0-50 chars
                let bar = "█".repeat(bar_len.min(50));

                let threshold = if is_recording {
                    MIN_SCORE_RECORDING
                } else {
                    MIN_SCORE_WAKE
                };
                print!("\rVol: [{:<50}] Score: {:.2}, {:.2}", bar, score, threshold);
                use std::io::{self, Write};
                io::stdout().flush().unwrap();

                frame_count = 0;
            }

            if !is_recording {
                let rms = calculate_rms(&frame_f32);
                let zcr = calculate_zcr(&frame_f32);

                if score > MIN_SCORE_WAKE && rms > 0.00 && zcr < 60 {
                    println!("🔥 VOICE DETECTED! (Score: {:.2})", score);
                    println!("🎤 Starting new recording...");
                    is_recording = true;
                    recording_buffer.extend(pre_roll.drain(..))
                }
            } else {
                recording_buffer.extend_from_slice(&frame_f32);

                if score > MIN_SCORE_RECORDING {
                    // Reset the "Hang Time" timer
                    silence_frames = 0;
                } else {
                    silence_frames += 1;
                }

                // After 1 sec of silence or up to 25 secs of talking
                if silence_frames >= MAX_SILENCE_FRAMES || recording_buffer.len() > 400000 {
                    println!(
                        "✅ Phrase complete. Total samples: {}",
                        recording_buffer.len()
                    );

                    // Send to whisper at least 1 second of audio
                    if recording_buffer.len() > 16000 {
                        let audio_to_process = std::mem::take(&mut recording_buffer);

                        tx_whisper.send(audio_to_process).ok();
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
