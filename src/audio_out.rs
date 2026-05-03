use log::{error, info};
use piper_rs::Piper;
use rodio::MixerDeviceSink;
use rodio::stream::DeviceSinkBuilder;
use rodio::{Player, buffer::SamplesBuffer};
use std::env;
use std::num::{NonZeroU16, NonZeroU32};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc::{self, Sender};

pub struct AudioEngine {
    pub tx: Sender<String>,
    player: Arc<Player>,
    _mixer_sink: Arc<MixerDeviceSink>, // Hold this or no audio output
}

impl AudioEngine {
    pub fn new() -> Self {
        let (tx, mut rx) = mpsc::channel::<String>(100);

        let mixer_sink =
            Arc::new(DeviceSinkBuilder::open_default_sink().expect("Could not open audio device"));

        let player = Arc::new(Player::connect_new(mixer_sink.mixer()));
        let player_clone = player.clone();

        std::thread::spawn(move || {
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

            info!("🔈 Speech output is ready.");

            while let Some(text) = rx.blocking_recv() {
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

                        player_clone.append(source);
                        player_clone.set_volume(1.0);
                        player_clone.play();
                    }
                    Err(e) => {
                        error!("Failed to say '{}'. Error: {:?}", text, e);
                    }
                }
            }
        });

        Self {
            tx,
            player: player.clone(),
            _mixer_sink: mixer_sink.clone(),
        }
    }

    pub fn is_active(&self) -> bool {
        // Paused is just waiting for data - not the same as stop
        !self.player.is_paused() && !self.player.empty()

        // To determine if silent
        // self.player.is_paused() || self.player.empty()
    }

    pub fn stop_audio(&self) {
        self.player.set_volume(0.0);
        self.player.stop();
    }
}
