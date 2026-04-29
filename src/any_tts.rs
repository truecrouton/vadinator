use any_tts::{ModelType, SynthesisRequest, TtsConfig, load_model};
use rodio::stream::DeviceSinkBuilder;
use rodio::{Player, buffer::SamplesBuffer};
use std::num::{NonZeroU16, NonZeroU32};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;

pub fn start_speech_worker(rx: Receiver<String>, is_speaking: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        // 1. New Rodio 0.22 Setup
        // OutputStream::try_default() is now open_default_sink()
        let mixer_sink =
            DeviceSinkBuilder::open_default_sink().expect("Could not open audio device");

        // Sink::try_new() is now Player::connect_new()
        let player = Player::connect_new(mixer_sink.mixer());

        // 2. Load the Candle-based Kokoro model (Pure Rust)
        let model =
            load_model(TtsConfig::new(ModelType::Kokoro).with_model_path("./models/Kokoro"))
                .expect("Failed to load TTS model");

        println!("🎙️ TTS Worker Online (Rodio 0.22)");

        while let Ok(text) = rx.recv() {
            // Tell the system we are about to make noise
            is_speaking.store(true, Ordering::SeqCst);

            if text.trim().is_empty() {
                continue;
            }

            // 3. Synthesize
            match model.synthesize(
                &SynthesisRequest::new(text)
                    .with_temperature(0.7)
                    .with_cfg_scale(2.0),
            ) {
                Ok(audio) => {
                    let source = SamplesBuffer::new(
                        NonZeroU16::new(1).unwrap(), // Channels
                        NonZeroU32::new(audio.sample_rate).unwrap(),
                        audio.samples,
                    );

                    // .append() is now on the Player
                    player.append(source);
                    player.sleep_until_end();
                }
                Err(e) => eprintln!("TTS Error: {}", e),
            }

            // 3. Done speaking
            is_speaking.store(false, Ordering::SeqCst);
        }
    });
}
