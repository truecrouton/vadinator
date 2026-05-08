use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::str::FromStr;
use std::sync::atomic::AtomicI64;
use tokio::runtime::Handle;

#[derive(sqlx::FromRow, Serialize, Deserialize, Clone, Debug)]
pub struct ChatMessage {
    pub role: String,
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

        Ok(Self {
            pool,
            context_id: AtomicI64::new(0),
            runtime: handle,
        })
    }

    async fn _current_context(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
        let (context_id,): (i64,) = sqlx::query_as(
            "SELECT IFNULL(MAX(context_id), 0) as context_id FROM context WHERE active>0",
        )
        .fetch_one(pool)
        .await?;

        Ok(context_id)
    }

    pub async fn enter_context(
        &self,
        context: &str,
        system_prompt: &str,
    ) -> Result<i64, sqlx::Error> {
        let mut tr = self.pool.begin().await?;

        sqlx::query("UPDATE context SET deactivated=context_id WHERE context=$1 and deactivated=0")
            .bind(context)
            .execute(&mut *tr)
            .await?;

        let result = sqlx::query("INSERT INTO context (context, system_prompt) VALUES ($1, $2)")
            .bind(context)
            .bind(system_prompt)
            .execute(&mut *tr)
            .await?;

        tr.commit().await?;

        let new_id = result.last_insert_rowid();

        self.context_id
            .store(new_id, std::sync::atomic::Ordering::SeqCst);

        Ok(new_id)
    }

    pub fn enter_context_sync(&self, context: &str, system_prompt: &str) -> i64 {
        self.runtime.block_on(async {
            self.enter_context(context, system_prompt)
                .await
                .expect("Database failed to enter context.")
        })
    }

    pub async fn add_message(&self, role: &str, content: &str) -> Result<(), sqlx::Error> {
        let context_id = self.context_id.load(std::sync::atomic::Ordering::SeqCst);
        if context_id < 1 {
            return Err(sqlx::Error::Configuration(
                "Could not find current context.".into(),
            ));
        }

        let _ = sqlx::query("INSERT INTO message (role, content, context_id) VALUES ($1, $2, $3)")
            .bind(role)
            .bind(content)
            .bind(context_id)
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
        let context_id = self.context_id.load(std::sync::atomic::Ordering::SeqCst);
        if context_id < 1 {
            return Err(sqlx::Error::Configuration(
                "Could not find current context.".into(),
            ));
        }

        let (system_prompt,): (String,) =
            sqlx::query_as("SELECT system_prompt FROM context WHERE context_id=$1")
                .bind(context_id)
                .fetch_one(&self.pool)
                .await?;

        let mut messages = sqlx::query_as::<_, ChatMessage>(
            "SELECT role, content FROM message WHERE context_id=$1",
        )
        .bind(context_id)
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
}
