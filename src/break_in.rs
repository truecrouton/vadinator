use crate::piper_tts::AudioEngine;
use log::debug;
use std::sync::{
    Arc,
    mpsc::{self, Sender},
};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext};

pub fn start_break_in_worker(
    context: Arc<WhisperContext>,
    ae: Arc<AudioEngine>,
) -> Sender<Vec<f32>> {
    let (tx, rx) = mpsc::channel::<Vec<f32>>();

    std::thread::spawn(move || {
        let mut break_in_state = context.create_state().unwrap();
        while let Ok(audio_data) = rx.recv() {
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

            if transcript.to_lowercase().contains("stop") {
                debug!("🛑: {}", transcript);

                ae.stop_audio();
            }
        }
    });

    tx
}
