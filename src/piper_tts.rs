use log::{error, info};
use piper_rs::Piper;
use rodio::stream::DeviceSinkBuilder;
use rodio::{Player, buffer::SamplesBuffer};
use std::env;
use std::num::{NonZeroU16, NonZeroU32};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;

pub fn start_speech_worker(rx: Receiver<String>, is_speaking: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mixer_sink =
            DeviceSinkBuilder::open_default_sink().expect("Could not open audio device");

        let player = Player::connect_new(mixer_sink.mixer());

        // Initialize Piper
        // This looks for the .onnx and the .json automatically if you point it to the model file.
        let base_path = PathBuf::from("./models");
        let config_file = env::var("PIPER_MODEL_CONFIG")
            .expect("Missing: PIPER_MODEL_CONFIG not set in your vadinator.env file.");
        let onnx_file = config_file.replace(".onnx.json", ".onnx");
        let config_path = base_path.join(config_file);
        let onnx_path = base_path.join(onnx_file);
        let speaker_id: Option<i64> = Some(0);
        let mut piper = Piper::new(Path::new(&onnx_path), Path::new(&config_path)).unwrap();

        info!("🔈 piper-rs is ready.");

        while let Ok(text) = rx.recv() {
            is_speaking.store(true, Ordering::SeqCst);

            if text.trim().is_empty() {
                continue;
            }

            let ignored_chars = ['*', '#'];
            let filtered_text: String = text
                .chars()
                .filter(|&c| !ignored_chars.contains(&c))
                .collect();

            match piper.create(
                &filtered_text,
                false,      // raw? false
                speaker_id, // speaker index
                None,       // length_scale (speed)
                None,       // noise_scale
                None,
            ) {
                Ok((samples, sample_rate)) => {
                    // Success so play the audio
                    let source = SamplesBuffer::new(
                        NonZeroU16::new(1).unwrap(), // Channels
                        NonZeroU32::new(sample_rate).unwrap(),
                        samples,
                    );

                    player.append(source);
                    player.sleep_until_end();
                }
                Err(e) => {
                    error!("Failed to say '{}'. Error: {:?}", text, e);
                }
            }

            // 3. Done speaking
            is_speaking.store(false, Ordering::SeqCst);
        }
    });
}
