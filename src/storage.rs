use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::prelude::FromRow;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::fs;
use std::str::FromStr;
use std::sync::atomic::AtomicI64;
use tokio::runtime::Handle;

#[derive(FromRow, Serialize, Deserialize, Clone, Debug)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, FromRow)]
pub struct StashRow {
    pub source: String,
    pub source_type: String,
    pub content: String,
}

pub struct Storage {
    pub pool: SqlitePool,
    context_id: AtomicI64,
    runtime: Handle,
}

impl Storage {
    pub async fn new() -> Result<Self, sqlx::Error> {
        let handle = Handle::current();

        let connection_options = SqliteConnectOptions::from_str("sqlite://data/vadinator.db")?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(connection_options)
            .await
            .expect("Failed to connect to database.");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("Migrations failed.");

        let context_id = Self::enter_context(&pool, None)
            .await
            .expect("Failed to enter chat context.");

        Ok(Self {
            pool,
            context_id: AtomicI64::new(context_id),
            runtime: handle,
        })
    }

    // Start a new context
    async fn enter_context(
        pool: &SqlitePool,
        mut system_prompt: Option<String>,
    ) -> Result<i64, sqlx::Error> {
        let mut tr = pool.begin().await?;

        sqlx::query("UPDATE context SET deactivated=context_id WHERE deactivated=0")
            .execute(&mut *tr)
            .await?;

        if system_prompt.is_none() {
            let default_prompt = fs::read_to_string("system_prompt.txt")
                .unwrap_or("You are a helpful assistant.".to_string());
            system_prompt = Some(default_prompt);
        }
        let result = sqlx::query("INSERT INTO context (system_prompt) VALUES ($1)")
            .bind(system_prompt)
            .execute(&mut *tr)
            .await?;
        let new_id = result.last_insert_rowid();

        tr.commit().await?;

        Ok(new_id)
    }

    pub async fn add_message(&self, role: &str, content: &str) -> Result<(), sqlx::Error> {
        let _ = sqlx::query("INSERT INTO message (role, content, context_id) VALUES ($1, $2, $3)")
            .bind(role)
            .bind(content)
            .bind(self.context_id.load(std::sync::atomic::Ordering::SeqCst))
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub fn add_message_sync(&self, role: &str, content: &str) {
        self.runtime.block_on(async {
            self.add_message(role, content)
                .await
                .expect("DB: Could not add message.")
        })
    }

    pub async fn get_payload(&self) -> Result<Vec<ChatMessage>, sqlx::Error> {
        let (system_prompt,): (String,) =
            sqlx::query_as("SELECT system_prompt FROM context WHERE context_id=$1")
                .bind(self.context_id.load(std::sync::atomic::Ordering::SeqCst))
                .fetch_one(&self.pool)
                .await?;

        let mut messages = sqlx::query_as::<_, ChatMessage>(
            "SELECT role, content FROM message WHERE context_id=$1 ORDER BY message_id ASC",
        )
        .bind(self.context_id.load(std::sync::atomic::Ordering::SeqCst))
        .fetch_all(&self.pool)
        .await?;

        messages.insert(
            0,
            ChatMessage {
                role: "system".to_string(),
                content: system_prompt,
            },
        );

        Ok(messages)
    }

    pub fn get_payload_sync(&self) -> Vec<ChatMessage> {
        self.runtime.block_on(async {
            self.get_payload()
                .await
                .expect("DB: could not get payload.")
        })
    }

    pub async fn synthesize_message(&self, topic: &str) -> Result<String, sqlx::Error> {
        let stashed:Vec<StashRow> = sqlx::query_as(
            "SELECT source, source_type, content FROM stash WHERE context_id=$1 and topic=$2 ORDER BY stash_id ASC"
        )
        .bind(self.context_id.load(std::sync::atomic::Ordering::SeqCst))
        .bind(topic)
        .fetch_all(&self.pool)
        .await?;

        let mut xml = String::new();
        xml.push_str("  <system_information topic=\"");
        xml.push_str(&Self::escape_xml(&topic));
        xml.push_str("\">\n");

        for stash in stashed {
            if let Ok(json) = serde_json::from_str::<Value>(&stash.content) {
                if let Some(obj) = json.as_object() {
                    xml.push_str("  <data source=\"");
                    xml.push_str(&Self::escape_xml(&stash.source));
                    xml.push_str(" source_type=\"");
                    xml.push_str(&Self::escape_xml(&stash.source_type));
                    xml.push_str("\">\n");
                    for (key, value) in obj {
                        xml.push_str("    <");
                        xml.push_str(&Self::escape_xml(key));
                        xml.push_str("\">");
                        match value {
                            Value::String(s) => {
                                xml.push_str(&Self::escape_xml(s));
                            }
                            Value::Number(n) => {
                                xml.push_str(&n.to_string());
                            }
                            Value::Bool(b) => {
                                xml.push_str(&b.to_string());
                            }
                            Value::Null => {
                                xml.push_str("null");
                            }
                            // Ignore Arrays and Objects (nested json)
                            Value::Array(_) | Value::Object(_) => {
                                continue;
                            }
                        }
                        xml.push_str("</");
                        xml.push_str(&Self::escape_xml(key));
                        xml.push_str("\">");
                    }
                    xml.push_str("  </data>\n");
                }
            }
        }

        xml.push_str("</system_information>\n");

        // Wrap instruction in its own tag
        xml.push_str("[INSTRUCTIONS]\n");
        xml.push_str("  Review the system data and report notable items to the user.\n");
        xml.push_str("  Make sure your report is succinct while also ensuring some context is provided to the user.\n");
        xml.push_str("  If anything seems out of the ordinary provide a succinct and useful recommendation.\n");

        println!("😝 XML: {}", xml);

        Ok(xml)
    }

    fn escape_xml(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    pub async fn stash_json_content(
        &self,
        source: &str,
        source_type: &str,
        topic: &str,
        content: &str,
    ) -> Result<(), sqlx::Error> {
        let _ = sqlx::query("INSERT INTO stash (source, source_type, topic, content, context_id) VALUES ($1, $2, $3, $4, $5)")
            .bind(source)
            .bind(source_type)
            .bind(topic)
            .bind(content)
            .bind(self.context_id.load(std::sync::atomic::Ordering::SeqCst))
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}
