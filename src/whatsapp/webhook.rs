use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde::Deserialize;
use sqlx::PgPool;

use crate::{
    error::{AppError, Result},
    state::AppState,
    whatsapp::{IncomingMessage, MessageContent, WebhookPayload},
};

pub fn router() -> Router<AppState> {
    Router::new().route("/webhook", get(verify).post(receive))
}

#[derive(Debug, Deserialize)]
struct VerifyParams {
    #[serde(rename = "hub.mode")]
    mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    verify_token: Option<String>,
    #[serde(rename = "hub.challenge")]
    challenge: Option<String>,
}

/// GET /webhook: Meta calls this once when you register the webhook URL.
/// We echo back the challenge only if the verify token matches ours.
async fn verify(State(state): State<AppState>, Query(params): Query<VerifyParams>) -> Response {
    let token_ok =
        params.verify_token.as_deref() == Some(state.config.whatsapp_verify_token.as_str());

    if params.mode.as_deref() == Some("subscribe") && token_ok {
        if let Some(challenge) = params.challenge {
            return (StatusCode::OK, challenge).into_response();
        }
    }

    tracing::warn!("webhook verification failed");
    StatusCode::FORBIDDEN.into_response()
}

/// POST /webhook: every incoming WhatsApp event lands here.
/// We must answer 200 fast, so real work is spawned into the background.
async fn receive(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode> {
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok());

    if !super::signature::verify_signature(&state.config.whatsapp_app_secret, &body, signature) {
        tracing::warn!("rejected webhook call with invalid signature");
        return Err(AppError::Unauthorized);
    }

    let payload: WebhookPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            // Answer 200 anyway so Meta does not keep retrying a payload we cannot read.
            tracing::warn!(error = %e, "could not parse webhook body");
            return Ok(StatusCode::OK);
        }
    };

    for message in payload.messages() {
        // Meta may deliver the same message more than once; handle each id only once.
        if !claim_message(&state.db, &message.id).await? {
            tracing::debug!(id = %message.id, "duplicate message ignored");
            continue;
        }

        let task_state = state.clone();
        tokio::spawn(async move {
            let id = message.id.clone();
            if let Err(e) = handle_incoming(task_state, message).await {
                tracing::error!(id = %id, error = %e, "failed to handle incoming message");
            }
        });
    }

    Ok(StatusCode::OK)
}

/// Record the message id. Returns true if this is the first time we see it.
async fn claim_message(db: &PgPool, message_id: &str) -> Result<bool> {
    let result = sqlx::query(
        "INSERT INTO processed_messages (message_id) VALUES ($1) ON CONFLICT DO NOTHING",
    )
    .bind(message_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// TEMPORARY handler used until Phase 6 plugs in the full voice pipeline.
/// It proves the whole WhatsApp plumbing works: signature, parsing, media download, replies.
async fn handle_incoming(state: AppState, msg: IncomingMessage) -> Result<()> {
    let wa = &state.whatsapp;

    match msg.content {
        MessageContent::Audio { media_id, .. } => {
            wa.mark_read(&msg.id).await.ok();
            let (bytes, mime) = wa.download_media(&media_id).await?;
            let reply = format!(
                "Voice note received ({} KB, {mime}). Understanding it arrives in the next phases.",
                bytes.len() / 1024
            );
            wa.send_text(&msg.from, &reply).await?;
        }
        MessageContent::Text(text) => {
            wa.send_text(&msg.from, &format!("Received: {text}")).await?;
        }
        MessageContent::ButtonReply { id, .. } => {
            wa.send_text(&msg.from, &format!("Button tapped: {id}")).await?;
        }
        MessageContent::Unsupported(kind) => {
            tracing::debug!(%kind, "unsupported message type");
            wa.send_text(&msg.from, "Please send a voice note.").await?;
        }
    }

    Ok(())
}