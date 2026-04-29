use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Message {
    pub role: String,
    pub content: String,
}

pub struct ChatHistory {
    system_prompt: Message,
    messages: Vec<Message>,
    max_history: usize,
}

impl ChatHistory {
    pub fn new(system: &str, limit: usize) -> Self {
        Self {
            system_prompt: Message {
                role: "system".into(),
                content: system.into(),
            },
            messages: Vec::new(),
            max_history: limit,
        }
    }

    pub fn add_message(&mut self, role: &str, content: &str) {
        self.messages.push(Message {
            role: role.into(),
            content: content.into(),
        });

        // If we exceed the limit, remove the oldest user/assistant pair
        if self.messages.len() > self.max_history {
            self.messages.drain(0..2); // Remove oldest exchange
        }
    }

    pub fn get_payload(&self) -> Vec<Message> {
        let mut payload = vec![self.system_prompt.clone()];
        payload.extend(self.messages.clone());
        payload
    }
}
