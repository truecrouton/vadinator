use crate::audio_out::AudioEngine;
use crate::conv_engine::ConversationEngine;
use log::debug;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc::{self, Sender};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext};

pub struct BreakInEngine {
    pub tx: Sender<Vec<f32>>,
    stop_processing: Arc<AtomicBool>,
}

impl BreakInEngine {
    pub fn new(
        context: Arc<WhisperContext>,
        ae: Arc<AudioEngine>,
        ce: Arc<ConversationEngine>,
    ) -> Self {
        let stop_processing = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = mpsc::channel::<Vec<f32>>(100);

        let clone_stop_processing = stop_processing.clone();
        std::thread::spawn(move || {
            let mut break_in_state = context.create_state().unwrap();
            while let Some(audio_data) = rx.blocking_recv() {
                if clone_stop_processing.load(Ordering::Relaxed) {
                    continue;
                }

                let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

                // Disable the "Standard" Whisper chatter
                params.set_print_special(false);
                params.set_print_progress(false);
                params.set_print_realtime(false);
                params.set_print_timestamps(false);
                params.set_suppress_blank(true);

                break_in_state
                    .full(params, &audio_data[..])
                    .expect("Break-in transcription failed");

                let mut transcript = String::new();
                for segment in break_in_state.as_iter() {
                    if let Ok(text) = segment.to_str() {
                        transcript.push(' ');
                        transcript.push_str(text.trim());
                    }
                }
                debug!("😱 Break-in transcription: {}", transcript);

                let has_stop_word = transcript
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|word| word.eq_ignore_ascii_case("ding"));
                if has_stop_word {
                    debug!("🛑 Stop detected: {}", transcript);
                    ce.stop();
                    ae.stop_audio();
                }
            }
        });

        Self {
            stop_processing,
            tx,
        }
    }

    pub fn pause(&self) {
        self.stop_processing.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.stop_processing.store(false, Ordering::Relaxed);
    }
}
