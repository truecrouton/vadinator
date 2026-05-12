use log::{error, info};
use piper_rs::Piper;
use rodio::MixerDeviceSink;
use rodio::stream::DeviceSinkBuilder;
use rodio::{Player, buffer::SamplesBuffer};
use std::env;
use std::num::{NonZeroU16, NonZeroU32};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc::{self, Sender};

enum Transcript {
    Text(String),
    Beginning,
}

pub struct AudioEngine {
    tx: Sender<Transcript>,
    player: Arc<Player>,
    _mixer_sink: Arc<MixerDeviceSink>, // Hold this or no audio output
    is_stopped: Arc<AtomicBool>,
}

impl AudioEngine {
    pub fn new() -> Self {
        let (tx, mut rx) = mpsc::channel::<Transcript>(100);
        let is_stopped = Arc::new(AtomicBool::new(true));

        let mixer_sink =
            Arc::new(DeviceSinkBuilder::open_default_sink().expect("Could not open audio device"));

        let player = Arc::new(Player::connect_new(mixer_sink.mixer()));
        let player_clone = player.clone();

        let is_stopped_clone = is_stopped.clone();
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

            while let Some(transcript) = rx.blocking_recv() {
                match transcript {
                    Transcript::Beginning => {
                        is_stopped_clone.store(false, Ordering::Relaxed);
                        player_clone.set_volume(1.0);
                    }
                    Transcript::Text(text) => {
                        if is_stopped_clone.load(Ordering::Relaxed) {
                            player_clone.set_volume(0.0);
                            continue;
                        } else {
                            player_clone.set_volume(1.0);
                        }
                        if text.trim().is_empty() {
                            continue;
                        }

                        let ignored_chars = ['*', '#'];
                        let emoji_less = Self::remove_emojis(&text);
                        let filtered_text: String = emoji_less
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
                                //player_clone.set_volume(1.0);
                                player_clone.play();
                            }
                            Err(e) => {
                                error!("Failed to say '{}'. Error: {:?}", text, e);
                            }
                        }
                    }
                }
            }
        });

        Self {
            tx,
            player: player.clone(),
            _mixer_sink: mixer_sink.clone(),
            is_stopped,
        }
    }

    pub fn is_active(&self) -> bool {
        // Paused is just waiting for data - not the same as stop
        !self.player.is_paused() && !self.player.empty()

        // To determine if silent
        // self.player.is_paused() || self.player.empty()
    }

    pub fn stop_audio(&self) {
        self.is_stopped.store(true, Ordering::Relaxed);
        self.player.set_volume(0.0);
        self.player.stop();
    }

    pub async fn buffer(&self, text: String, transcript_start: bool) {
        if transcript_start {
            let _ = self.tx.send(Transcript::Beginning).await;
        }

        let _ = self.tx.send(Transcript::Text(text)).await;
    }

    pub fn blocking_buffer(&self, text: String, transcript_start: bool) {
        if transcript_start {
            self.tx.blocking_send(Transcript::Beginning).ok();
        }

        self.tx.blocking_send(Transcript::Text(text)).ok();
    }

    fn remove_emojis(input: &str) -> String {
        let mut output = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();

        while let Some(c) = chars.next() {
            let u = c as u32;

            // The "Symbol Continents": where 99% of emojis live now and in the future.
            if matches!(u, 0x1F000..=0x1Faff | 0x2600..=0x27BF ) {
                // It's a symbol! Now eat the (modifiers/glue)
                while let Some(&next_c) = chars.peek() {
                    let n = next_c as u32;
                    // Eat: ZWJ, Variation Selectors, Skin Tones, or more symbols
                    if matches!(n, 0x200D | 0xFE00..=0xFE0F | 0x1F3FB..=0x1F3FF | 0x1F000..=0x1Faff)
                    {
                        chars.next();
                    } else {
                        break;
                    }
                }
            } else {
                output.push(c);
            }
        }
        output
    }
}
