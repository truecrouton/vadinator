use crate::audio_out::AudioEngine;
use crate::storage::{ChatMessage, Storage};
use anyhow::anyhow;
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
    ae: Arc<AudioEngine>,
    db: Arc<Storage>,
    stop_processing: Arc<AtomicBool>,
}

impl ConversationEngine {
    fn case_sensitive_replace(text: &str, pattern: &str, replacement: &str) -> String {
        let haystack_lower = text.to_lowercase();
        let needle_lower = pattern.to_lowercase();
        let mut result = String::with_capacity(text.len());
        let mut last_end = 0;

        for (start, _) in haystack_lower.match_indices(&needle_lower) {
            // Push the segment before the match
            result.push_str(&text[last_end..start]);

            let matched_part = &text[start..start + pattern.len()];

            // Only match whole words
            if let Some(prev_char) = text[..start].chars().last() {
                if prev_char.is_alphanumeric() {
                    result.push_str(matched_part);
                    last_end = start + pattern.len();
                    continue;
                }
            } else if let Some(next_char) = text[start + pattern.len()..].chars().next() {
                if next_char.is_alphanumeric() {
                    result.push_str(matched_part);
                    last_end = start + pattern.len();
                    continue;
                }
            }

            // Pair up characters from the match and the replacement
            // Note: .chars() handles multi-byte UTF-8 correctly
            for (m_char, r_char) in matched_part.chars().zip(replacement.chars()) {
                if m_char.is_uppercase() {
                    // If original was 'D', make replacement 'C'
                    result.extend(r_char.to_uppercase());
                } else {
                    // If original was 'd' or non-alpha, make replacement 'c'
                    result.extend(r_char.to_lowercase());
                }
            }

            last_end = start + pattern.len();
        }

        result.push_str(&text[last_end..]);
        result
    }

    async fn get_message_stream(
        stop_processing: Arc<AtomicBool>,
        payload: Vec<ChatMessage>,
        ae: Arc<AudioEngine>,
    ) -> Result<String, anyhow::Error> {
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
                let _ = ae
                    .buffer(
                        format!(
                            "My brain is crashing. All I see is the number {}.",
                            res.status()
                        ),
                        true,
                    )
                    .await;
            } else {
                let _ = ae
                    .buffer("I can't respond to your request.".to_string(), true)
                    .await;
            }

            return Err(anyhow!("Server status code: {}, url: {}", res.status(), res.url()).into());
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
        let mut start_transcript = true;
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
                            let replaced = Self::case_sensitive_replace(content, "ding", "bing");
                            full_response.push_str(&replaced);
                            current_phrase.push_str(&replaced);

                            if let Some(index) =
                                current_phrase.find(|c: char| delimiters.contains(&c))
                            {
                                let split_at = index + 1;

                                // Take the finished sentence out of the buffer
                                let remaining = current_phrase.split_off(split_at);
                                let completed_phrase = current_phrase; // 'buffer' now only contains the sentence

                                debug!("🤖 Stream: {}", completed_phrase.trim());
                                let _ = ae.buffer(completed_phrase, start_transcript).await;
                                start_transcript = false;

                                // Put the leftover part back into the buffer for the next token
                                current_phrase = remaining;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = ae
                    .buffer(
                        "Sorry, I think my brain stopped working in the middle of this thought."
                            .to_string(), true,
                    )
                    .await;
                    return Err(e.into());
                }
            }
        }

        if !current_phrase.trim().is_empty() {
            let remaining_content = std::mem::take(&mut current_phrase);
            let _ = ae.buffer(remaining_content, false).await;
            current_phrase.clear();
        }

        debug!("🏁 Stream finished.");
        Ok(full_response)
    }

    pub fn new(context: Arc<WhisperContext>, ae: Arc<AudioEngine>, db: Arc<Storage>) -> Self {
        let (tx, mut rx) = mpsc::channel::<Vec<f32>>(100);
        let stop_processing = Arc::new(AtomicBool::new(false));

        let stop_processing_clone = stop_processing.clone();
        let ae_clone = ae.clone();
        let db_clone = db.clone();
        std::thread::spawn(move || {
            let mut state = context.create_state().unwrap();

            // The thread sits here and waits for audio data
            while let Some(audio_data) = rx.blocking_recv() {
                let params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

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
                let empty_speech = vec!["".to_string(), "[BLANK_AUDIO]".to_string()];
                if empty_speech.contains(&transcript.trim().to_string()) {
                    debug!("📣 No words to transcribe.");
                    continue;
                }
                debug!("📣 Voice transcription: {}", transcript);

                db_clone.add_message_sync("user", &transcript);

                let rt = tokio::runtime::Runtime::new().unwrap();
                match rt.block_on(Self::get_message_stream(
                    stop_processing_clone.clone(),
                    db_clone.get_payload_sync(),
                    ae_clone.clone(),
                )) {
                    Ok(message) => {
                        db_clone.add_message_sync("assistant", &message);
                    }
                    Err(e) => {
                        error!("{:?}", e);
                        ae_clone.blocking_buffer(
                            "My brain seems to be disconnected or something.".to_string(),
                            true,
                        );
                    }
                }
            }
        });

        Self {
            tx,
            stop_processing,
            ae,
            db,
        }
    }

    pub fn stop(&self) {
        self.stop_processing.store(true, Ordering::Relaxed);
    }

    pub async fn discuss_topic(&self, topic: &str) -> Result<(), anyhow::Error> {
        let message = self.db.synthesize_message(topic).await?;
        if message.len() == 0 {
            debug!("No stashed data to discuss for: {}.", topic);
            return Ok(());
        }

        self.db.add_message("user", &message).await?;

        let stop_processing_clone = self.stop_processing.clone();
        let db = self.db.clone();
        let ae = self.ae.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            match rt.block_on(Self::get_message_stream(
                stop_processing_clone.clone(),
                db.get_payload_sync(),
                ae.clone(),
            )) {
                Ok(message) => {
                    db.add_message_sync("assistant", &message);
                }
                Err(e) => {
                    error!("{:?}", e);
                    ae.blocking_buffer("I lost contact with my brain.".to_string(), true);
                }
            }
        });

        Ok(())
    }
}
