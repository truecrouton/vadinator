use log::{error, info};
use piper_rs::Piper;
use rodio::stream::DeviceSinkBuilder;
use rodio::{Player, buffer::SamplesBuffer};
use std::num::{NonZeroU16, NonZeroU32};
use std::path::Path;
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
        let config_path = "./models/en_US-hfc_female-medium.onnx.json";
        let onnx_path = config_path.replace(".onnx.json", ".onnx");
        let speaker_id: Option<i64> = Some(0);
        let mut piper = Piper::new(Path::new(&onnx_path), Path::new(&config_path)).unwrap();

        info!("🔈 piper-rs is ready.");

        while let Ok(text) = rx.recv() {
            is_speaking.store(true, Ordering::SeqCst);

            if text.trim().is_empty() {
                continue;
            }

            match piper.create(
                &text, false,      // raw? false
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
