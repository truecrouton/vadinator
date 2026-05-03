use crate::audio_out::AudioEngine;
use crate::chat_history::{ChatHistory, Message};
use futures_util::StreamExt;
use log::{debug, error};
use reqwest::Client;
use serde_json::Value;
use serde_json::json;
use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc::{self, Sender};
use tokio::time::{Duration, timeout};
use tokio_util::codec::{FramedRead, LinesCodec};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext};

pub struct ConversationEngine {
    pub tx: Sender<Vec<f32>>,
    stop_processing: Arc<AtomicBool>,
}

impl ConversationEngine {
    async fn get_message_stream(
        stop_processing: Arc<AtomicBool>,
        payload: Vec<Message>,
        speaker_tx: Sender<String>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let client = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(3))
            .timeout(std::time::Duration::from_secs(300))
            .build()?;

        let url = env::var("SERVER_URL")
            .expect("Missing: SERVER_URL not set in your vadinator.env file.");

        let body = json!({
            "messages": payload,
            "stream": true
        });

        let res = client.post(url).json(&body).send().await?;

        if !res.status().is_success() {
            error!("Server status code: {}, url: {}", res.status(), res.url());

            if res.status().is_server_error() {
                let _ = speaker_tx
                    .send(
                        format!(
                            "My brain is crashing. All I see is the number {}.",
                            res.status()
                        )
                        .to_string(),
                    )
                    .await;
            } else {
                let _ = speaker_tx
                    .send("I can't respond to your request.".to_string())
                    .await;
            }

            return Err(format!("Server status code: {}, url: {}", res.status(), res.url()).into());
        }

        // Convert the reqwest Body into a AsyncRead-compatible byte stream
        let byte_stream = res
            .bytes_stream()
            .map(|result| result.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));

        let sync_wrapper = tokio_util::io::StreamReader::new(byte_stream);

        // Use LinesCodec to automatically buffer and split by \n
        let mut lines = FramedRead::new(sync_wrapper, LinesCodec::new());

        let mut full_response = String::new();
        let mut current_phrase = String::new();
        let delimiters = ['.', '!', '?'];

        stop_processing.store(false, Ordering::Relaxed);
        while let Some(l) = lines.next().await {
            match timeout(Duration::from_secs(3), std::future::ready(l)).await {
                Ok(line) => {
                    if stop_processing.load(Ordering::Relaxed) {
                        break;
                    }

                    let line = line?;
                    let line = line.trim();

                    if line.is_empty() || !line.starts_with("data: ") {
                        continue;
                    }

                    let data = &line[6..];

                    if data == "[DONE]" {
                        break;
                    }

                    if let Ok(json) = serde_json::from_str::<Value>(data) {
                        if let Some(content) = json["choices"][0]["delta"]["content"].as_str() {
                            full_response.push_str(content);
                            current_phrase.push_str(content);

                            if let Some(index) =
                                current_phrase.find(|c: char| delimiters.contains(&c))
                            {
                                let split_at = index + 1;

                                // Take the finished sentence out of the buffer
                                let remaining = current_phrase.split_off(split_at);
                                let completed_phrase = current_phrase; // 'buffer' now only contains the sentence

                                debug!("🤖 Stream: {}", completed_phrase.trim());
                                let _ = speaker_tx.send(completed_phrase).await;

                                // Put the leftover part back into the buffer for the next token
                                current_phrase = remaining;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = speaker_tx
                    .send(
                        "Sorry, I think my brain stopped working in the middle of this thought."
                            .to_string(),
                    )
                    .await;
                    return Err(e.into());
                }
            }
        }

        if !current_phrase.trim().is_empty() {
            let remaining_content = std::mem::take(&mut current_phrase);
            let _ = speaker_tx.send(remaining_content).await;
            current_phrase.clear();
        }

        debug!("🏁 Stream finished.");
        Ok(full_response)
    }

    pub fn new(context: Arc<WhisperContext>, ae: Arc<AudioEngine>, system_prompt: &str) -> Self {
        let (tx, mut rx) = mpsc::channel::<Vec<f32>>(100);
        let mut history = ChatHistory::new(&system_prompt, 30);
        let stop_processing = Arc::new(AtomicBool::new(false));

        let stop_processing_clone = stop_processing.clone();
        std::thread::spawn(move || {
            let mut state = context.create_state().unwrap();

            // The thread sits here and waits for audio data
            while let Some(audio_data) = rx.blocking_recv() {
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
                debug!("📣 Voice transcription: {}", transcript);
                if transcript.trim() == "[BLANK_AUDIO]" {
                    continue;
                }

                history.add_message("user", &transcript);

                let rt = tokio::runtime::Runtime::new().unwrap();
                match rt.block_on(Self::get_message_stream(
                    stop_processing_clone.clone(),
                    history.get_payload(),
                    ae.tx.clone(),
                )) {
                    Ok(message) => {
                        history.add_message("assistant", &message);
                    }
                    Err(e) => {
                        error!("{:?}", e);
                        ae.tx
                            .blocking_send(
                                "My brain seems to be disconnected or something.".to_string(),
                            )
                            .ok();
                    }
                }
            }
        });

        Self {
            tx,
            stop_processing,
        }
    }

    pub fn stop(&self) {
        self.stop_processing.store(true, Ordering::Relaxed);
    }
}
