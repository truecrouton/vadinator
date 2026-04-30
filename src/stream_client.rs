use crate::chat_history::Message as PayloadMessage;
use futures_util::StreamExt;
use log::debug;
use log::error;
use reqwest::Client;
use serde_json::Value;
use serde_json::json;
use std::sync::mpsc::Sender;
use tokio::time::{Duration, timeout};
use tokio_util::codec::{FramedRead, LinesCodec};

pub async fn get_message_stream(
    payload: Vec<PayloadMessage>,
    speaker_tx: Sender<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    // llama.cpp server usually runs on 8080, Ollama on 11434
    let url = "http://localhost:8080/v1/chat/completions";

    let body = json!({
        //"prompt": format!("\n\n### User: {}\n### Assistant:", text),
        "messages": payload,
        "stream": true
    });

    let res = client.post(url).json(&body).send().await?;

    if !res.status().is_success() {
        error!("Server status code: {}, url: {}", res.status(), res.url());

        if res.status().is_server_error() {
            speaker_tx
                .send(
                    format!(
                        "My brain is crashing. All I see is the number {}.",
                        res.status()
                    )
                    .to_string(),
                )
                .ok();
        } else {
            speaker_tx
                .send("I think your request is somehow going to make my brain puke.".to_string())
                .ok();
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

    let mut current_phrase = String::new();
    let delimiters = ['.', '!', '?'];

    while let Some(l) = lines.next().await {
        match timeout(Duration::from_secs(3), std::future::ready(l)).await {
            Ok(line) => {
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
                        current_phrase.push_str(content);

                        if let Some(index) = current_phrase.find(|c: char| delimiters.contains(&c))
                        {
                            let split_at = index + 1;

                            // Take the finished sentence out of the buffer
                            let remaining = current_phrase.split_off(split_at);
                            let completed_phrase = current_phrase; // 'buffer' now only contains the sentence

                            debug!("Completed phrase: {}", completed_phrase.trim());
                            speaker_tx.send(completed_phrase).ok();

                            // Put the leftover part back into the buffer for the next token
                            current_phrase = remaining;
                        }
                    }
                }
            }
            Err(e) => {
                speaker_tx
                    .send(
                        "Sorry, I think my brain stopped working in the middle of this thought."
                            .to_string(),
                    )
                    .ok();
                return Err(e.into());
            }
        }
    }

    if !current_phrase.trim().is_empty() {
        let remaining_content = std::mem::take(&mut current_phrase);
        speaker_tx.send(remaining_content).ok();
        current_phrase.clear();
    }

    debug!("Stream finished.");
    Ok(())
}
