use crate::chat_history::Message as PayloadMessage;
use log::error;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize, Debug)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize, Debug)]
struct Choice {
    message: Message,
}

#[derive(Deserialize, Debug)]
struct Message {
    content: String,
}

pub async fn send_to_llama(payload: Vec<PayloadMessage>) -> Result<String, reqwest::Error> {
    let client = reqwest::Client::new();

    // llama.cpp server usually runs on 8080, Ollama on 11434
    let url = "http://localhost:8080/v1/chat/completions";

    let body = json!({
        //"prompt": format!("\n\n### User: {}\n### Assistant:", text),
        "messages": payload,
        "stream": false
    });

    let res = client.post(url).json(&body).send().await?;

    if res.status().is_success() {
        match res.json::<ChatResponse>().await {
            Ok(data) => {
                return Ok(data.choices[0].message.content.to_string());
            }
            Err(e) => {
                error!("{:?}", e);
                return Ok("Sorry, I'm getting gibberish from my brain.".to_string());
            }
        }
    } else if res.status().is_server_error() {
        // 500 errors
        error!("Server status code: {}, url: {}", res.status(), res.url());
        return Ok("My brain is crashing. All I see is the number 500.".to_string());
    } else {
        error!("Server status code: {}, url {}", res.status(), res.url());
        return Ok("I think your request is somehow going to make my brain puke.".to_string());
    }
}
