use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use crate::{
    config::Config,
    error::{AppError, Result},
    nlp::{prompt, IntentParser, ParseContext, ParsedCommand},
};

const MAX_ATTEMPTS: u64 = 3;

/// Intent parser for any OpenAI-style `/chat/completions` endpoint: Groq, Google Gemini
/// (OpenAI-compatible mode), OpenRouter, Ollama, OpenAI itself.
/// It asks for a JSON object, which every one of those supports, instead of tool calls,
/// which smaller free models handle less reliably.
pub struct OpenAiCompatClient {
    http: Client,
    url: String,
    api_key: String,
    model: String,
}

impl OpenAiCompatClient {
    pub fn new(config: &Config) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("failed to build HTTP client");

        Self {
            http,
            url: format!("{}/chat/completions", config.nlp_base_url),
            api_key: config.nlp_api_key.clone(),
            model: config.nlp_model.clone(),
        }
    }

    fn request_body(&self, transcript: &str, ctx: &ParseContext, json_mode: bool) -> Value {
        let mut body = json!({
            "model": self.model,
            "temperature": 0,
            // Generous, because some free models "think" before answering and that uses tokens too.
            "max_tokens": 1500,
            "messages": [
                { "role": "system", "content": prompt::system_prompt_json(ctx) },
                { "role": "user", "content": prompt::user_prompt(transcript) }
            ]
        });
        if json_mode {
            body["response_format"] = json!({ "type": "json_object" });
        }
        body
    }

    /// POST with a couple of retries for rate limits (free tiers hit these) and server trouble.
    async fn call(&self, body: &Value) -> Result<Value> {
        let mut last_error = AppError::Nlp("no attempt made".to_string());

        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2 * attempt)).await;
            }

            let resp = match self
                .http
                .post(&self.url)
                .bearer_auth(&self.api_key)
                .json(body)
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    last_error = AppError::Http(e);
                    continue;
                }
            };

            let status = resp.status();
            if status.is_success() {
                return Ok(resp.json::<Value>().await?);
            }

            let text = resp.text().await.unwrap_or_default();
            let short: String = text.chars().take(300).collect();
            last_error = AppError::Nlp(format!("{status}: {short}"));

            let retryable = status.as_u16() == 429 || status.is_server_error();
            if !retryable {
                break;
            }
        }

        Err(last_error)
    }
}

#[async_trait]
impl IntentParser for OpenAiCompatClient {
    async fn parse(&self, transcript: &str, ctx: &ParseContext) -> Result<ParsedCommand> {
        if transcript.trim().is_empty() {
            return Err(AppError::Nlp("empty transcript".to_string()));
        }

        let first = self.call(&self.request_body(transcript, ctx, true)).await;
        let response = match first {
            // A few providers or models reject response_format; ask again without it.
            Err(AppError::Nlp(ref msg)) if msg.starts_with("400") => {
                tracing::warn!("provider rejected JSON mode, retrying without it");
                self.call(&self.request_body(transcript, ctx, false)).await?
            }
            other => other?,
        };

        let content = extract_content(&response);
        Ok(ParsedCommand::from_json_text(&content))
    }
}

/// The assistant's text from a chat-completions response, minus any <think>...</think> block
/// that reasoning models print first.
fn extract_content(response: &Value) -> String {
    let content = response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("");

    strip_think(content).trim().to_string()
}

fn strip_think(text: &str) -> &str {
    match text.rfind("</think>") {
        Some(index) => &text[index + "</think>".len()..],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlp::Command;
    use chrono::NaiveDate;

    fn client() -> OpenAiCompatClient {
        OpenAiCompatClient {
            http: Client::new(),
            url: "http://localhost/v1/chat/completions".to_string(),
            api_key: "test-key".to_string(),
            model: "test-model".to_string(),
        }
    }

    fn ctx() -> ParseContext {
        ParseContext {
            today: NaiveDate::from_ymd_opt(2026, 11, 22).unwrap(),
            employee_names: vec![],
            party_names: vec![],
        }
    }

    #[test]
    fn request_asks_for_json_object() {
        let body = client().request_body("ali ko paanch sau ka maal becha", &ctx(), true);
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["messages"][0]["role"], "system");
        assert!(body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("ONLY one JSON object"));
        assert!(body["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("paanch sau"));
    }

    #[test]
    fn fallback_request_has_no_response_format() {
        let body = client().request_body("salam", &ctx(), false);
        assert!(body.get("response_format").is_none());
    }

    #[test]
    fn reads_content_from_chat_completion() {
        let response = json!({
            "choices": [{ "message": { "role": "assistant",
                "content": "{\"intent\":\"query_balance\",\"spoken_language\":\"ur\"}" } }]
        });
        let parsed = ParsedCommand::from_json_text(&extract_content(&response));
        assert_eq!(parsed.command, Command::QueryBalance);
        assert_eq!(parsed.language.as_deref(), Some("ur"));
    }

    #[test]
    fn strips_reasoning_block_before_json() {
        let response = json!({
            "choices": [{ "message": { "content":
                "<think>maybe {balance}?</think>\n{\"intent\":\"help\",\"spoken_language\":\"en\"}" } }]
        });
        let parsed = ParsedCommand::from_json_text(&extract_content(&response));
        assert_eq!(parsed.command, Command::Help);
    }

    #[test]
    fn empty_or_odd_response_becomes_unclear() {
        let response = json!({ "choices": [{ "message": { "content": null } }] });
        let parsed = ParsedCommand::from_json_text(&extract_content(&response));
        assert!(matches!(parsed.command, Command::Unclear { .. }));
    }
}