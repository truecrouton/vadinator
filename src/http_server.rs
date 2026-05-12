use crate::{audio_out::AudioEngine, conv_engine::ConversationEngine, storage::Storage};
use axum::{
    Json, Router,
    extract::State,
    response::{IntoResponse, Response},
    routing::post,
};
use axum_valid::Valid;
use log::{error, info};
use serde::Deserialize;
use serde_json::Value;
use std::{str::FromStr, sync::Arc};
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct AddMessage {
    #[validate(length(min = 1, max = 30))]
    pub context: String,
    pub message: Value,
}

#[derive(Deserialize, Validate)]
pub struct Discuss {
    #[validate(length(min = 1, max = 30))]
    pub topic: String,
}

#[derive(Deserialize, Validate)]
pub struct StashJsonContent {
    #[validate(length(min = 1, max = 30))]
    pub source: String,
    #[validate(length(min = 1, max = 30))]
    pub source_type: String,
    #[validate(length(min = 1, max = 30))]
    pub topic: String,
    pub content: Value,
}

#[derive(Clone)] // Must be Clone to work with State
pub struct HttpServer {
    ae: Arc<AudioEngine>,
    ce: Arc<ConversationEngine>,
    db: Arc<Storage>,
}

impl HttpServer {
    pub fn new(ae: Arc<AudioEngine>, ce: Arc<ConversationEngine>, db: Arc<Storage>) -> Self {
        Self { ae, ce, db }
    }

    pub async fn start_server(self, host_address: &str) {
        let app: Router = Router::new()
            .route("/add_message", post(Self::add_message))
            .route("/stash_content", post(Self::stash_content))
            .route("/discuss_topic", post(Self::discuss_topic))
            .with_state(self);

        let addr =
            std::net::SocketAddr::from_str(host_address).expect("Invalid HOST_ADDRESS provided.");
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                info!("Starting HTTP server at: {}", addr);
                axum::serve(listener, app)
                    .await
                    .expect("Failed to start HTTP server.");
            }
            Err(e) => {
                error!("Failed to bind to address: {}", e);
            }
        }
    }

    async fn add_message(
        State(state): State<HttpServer>,
        Valid(Json(payload)): Valid<Json<AddMessage>>,
    ) -> Response {
        let message = payload.message.to_string();
        if let Err(e) = state.db.add_message("user", &message, None).await {
            error!("Could not add message: {}", e);
        }

        "ok".into_response()
    }

    async fn stash_content(
        State(state): State<HttpServer>,
        Valid(Json(payload)): Valid<Json<StashJsonContent>>,
    ) -> Response {
        let json_string = payload.content.to_string();
        if let Err(e) = state
            .db
            .stash_content(
                &payload.source,
                &payload.source_type,
                &payload.topic,
                &json_string,
            )
            .await
        {
            error!("Could not stash content: {}", e);
        }

        "ok".into_response()
    }

    async fn discuss_topic(
        State(state): State<HttpServer>,
        Valid(Json(payload)): Valid<Json<Discuss>>,
    ) -> Response {
        if let Err(e) = state.ce.discuss_topic(&payload.topic).await {
            error!("Could not resume chat: {}", e);
            state
                .ae
                .buffer(
                    "I just received a message but I cannot chat about it.".to_string(),
                    true,
                )
                .await;
        }

        "ok".into_response()
    }
}
