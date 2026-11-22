use std::time::Duration;

use reqwest::{Client, Response};
use serde_json::{json, Value};

use crate::{
    config::Config,
    error::{AppError, Result},
    whatsapp::{BTN_CONFIRM_NO, BTN_CONFIRM_YES},
};

const GRAPH_BASE: &str = "https://graph.facebook.com";
/// WhatsApp text messages are limited to 4096 characters.
const MAX_TEXT_LEN: usize = 4000;
/// WhatsApp reply-button titles are limited to 20 characters.
const MAX_BUTTON_TITLE_LEN: usize = 20;
/// Whisper accepts files up to 25 MB; refuse bigger downloads early.
const MAX_MEDIA_BYTES: usize = 25 * 1024 * 1024;

/// Thin wrapper over the WhatsApp Cloud API: send text, send Yes/No buttons, download voice notes.
#[derive(Clone)]
pub struct WhatsAppClient {
    http: Client,
    access_token: String,
    phone_number_id: String,
    api_version: String,
}

impl WhatsAppClient {
    pub fn new(config: &Config) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");

        Self {
            http,
            access_token: config.whatsapp_access_token.clone(),
            phone_number_id: config.whatsapp_phone_number_id.clone(),
            api_version: config.whatsapp_api_version.clone(),
        }
    }

    fn messages_url(&self) -> String {
        format!("{GRAPH_BASE}/{}/{}/messages", self.api_version, self.phone_number_id)
    }

    async fn post_message(&self, body: Value) -> Result<()> {
        let resp = self
            .http
            .post(self.messages_url())
            .bearer_auth(&self.access_token)
            .json(&body)
            .send()
            .await?;
        check_status(resp).await?;
        Ok(())
    }

    /// Send a plain text message.
    pub async fn send_text(&self, to: &str, text: &str) -> Result<()> {
        self.post_message(json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": to,
            "type": "text",
            "text": { "preview_url": false, "body": truncate(text, MAX_TEXT_LEN) }
        }))
        .await
    }

    /// Send a message with two tap buttons (Yes / No). Taps come back as `ButtonReply`
    /// messages carrying `BTN_CONFIRM_YES` or `BTN_CONFIRM_NO`.
    pub async fn send_confirm_buttons(
        &self,
        to: &str,
        body: &str,
        yes_title: &str,
        no_title: &str,
    ) -> Result<()> {
        self.post_message(json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": to,
            "type": "interactive",
            "interactive": {
                "type": "button",
                "body": { "text": truncate(body, 1000) },
                "action": {
                    "buttons": [
                        { "type": "reply",
                          "reply": { "id": BTN_CONFIRM_YES, "title": truncate(yes_title, MAX_BUTTON_TITLE_LEN) } },
                        { "type": "reply",
                          "reply": { "id": BTN_CONFIRM_NO, "title": truncate(no_title, MAX_BUTTON_TITLE_LEN) } }
                    ]
                }
            }
        }))
        .await
    }

    /// Mark an incoming message as read (blue ticks), so the user knows it arrived.
    pub async fn mark_read(&self, message_id: &str) -> Result<()> {
        self.post_message(json!({
            "messaging_product": "whatsapp",
            "status": "read",
            "message_id": message_id
        }))
        .await
    }

    /// Download a voice note. Returns the raw bytes and the mime type reported by WhatsApp.
    /// Two calls are needed: first the media id is resolved to a short-lived URL, then the URL is fetched.
    pub async fn download_media(&self, media_id: &str) -> Result<(Vec<u8>, String)> {
        let meta_resp = self
            .http
            .get(format!("{GRAPH_BASE}/{}/{}", self.api_version, media_id))
            .bearer_auth(&self.access_token)
            .send()
            .await?;
        let meta: Value = check_status(meta_resp).await?.json().await?;

        let url = meta
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::WhatsApp("media lookup returned no url".to_string()))?;
        let mime = meta
            .get("mime_type")
            .and_then(Value::as_str)
            .unwrap_or("audio/ogg")
            .to_string();

        let file_resp = self
            .http
            .get(url)
            .bearer_auth(&self.access_token)
            .send()
            .await?;
        let bytes = check_status(file_resp).await?.bytes().await?;

        if bytes.len() > MAX_MEDIA_BYTES {
            return Err(AppError::WhatsApp(format!(
                "voice note too large ({} bytes)",
                bytes.len()
            )));
        }

        Ok((bytes.to_vec(), mime))
    }
}

/// Turn a non-2xx Meta response into an error that includes Meta's explanation.
async fn check_status(resp: Response) -> Result<Response> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    Err(AppError::WhatsApp(format!("{status}: {}", truncate(&text, 300))))
}

fn truncate(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncates_by_characters_not_bytes() {
        assert_eq!(truncate("ہاں", 2), "ہا");
        assert_eq!(truncate("abc", 10), "abc");
    }
}