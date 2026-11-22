use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use crate::{
    config::Config,
    error::{AppError, Result},
    nlp::{prompt, IntentParser, ParseContext, ParsedCommand},
};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const TOOL_NAME: &str = "record_command";
const MAX_ATTEMPTS: u64 = 3;

/// Claude API client that turns a transcript into a structured command.
/// The model is forced to call one tool, so the answer is always structured JSON.
pub struct ClaudeClient {
    http: Client,
    api_key: String,
    model: String,
}

impl ClaudeClient {
    pub fn new(config: &Config) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(45))
            .build()
            .expect("failed to build HTTP client");

        Self {
            http,
            api_key: config.anthropic_api_key.clone(),
            model: config.claude_model.clone(),
        }
    }

    fn request_body(&self, transcript: &str, ctx: &ParseContext) -> Value {
        json!({
            "model": self.model,
            "max_tokens": 700,
            "system": prompt::system_prompt(ctx),
            "tools": [tool_definition()],
            "tool_choice": { "type": "tool", "name": TOOL_NAME },
            "messages": [
                { "role": "user", "content": prompt::user_prompt(transcript) }
            ]
        })
    }

    /// POST with a couple of retries for rate limits and temporary server trouble.
    async fn call(&self, body: &Value) -> Result<Value> {
        let mut last_error = AppError::Nlp("no attempt made".to_string());

        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(800 * attempt)).await;
            }

            let resp = match self
                .http
                .post(API_URL)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", API_VERSION)
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
impl IntentParser for ClaudeClient {
    async fn parse(&self, transcript: &str, ctx: &ParseContext) -> Result<ParsedCommand> {
        if transcript.trim().is_empty() {
            return Err(AppError::Nlp("empty transcript".to_string()));
        }

        let body = self.request_body(transcript, ctx);
        let response = self.call(&body).await?;

        if let Some(input) = extract_tool_input(&response) {
            return Ok(ParsedCommand::from_value(input));
        }

        // Fallback: the model answered with text instead of the tool call.
        let text = extract_text(&response);
        Ok(ParsedCommand::from_json_text(&text))
    }
}

fn extract_tool_input(response: &Value) -> Option<Value> {
    response
        .get("content")?
        .as_array()?
        .iter()
        .find(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        .and_then(|block| block.get("input"))
        .cloned()
}

fn extract_text(response: &Value) -> String {
    response
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// The JSON schema the model must fill in. Field names match `nlp::intent::Command`.
fn tool_definition() -> Value {
    json!({
        "name": TOOL_NAME,
        "description": "Record the single business command found in the voice-note transcript.",
        "input_schema": {
            "type": "object",
            "properties": {
                "spoken_language": {
                    "type": "string",
                    "enum": ["ur", "pa", "ps", "sd", "en"],
                    "description": "Main language the speaker used."
                },
                "intent": {
                    "type": "string",
                    "enum": [
                        "record_transaction", "assign_task", "complete_task", "add_employee",
                        "query_summary", "query_balance", "query_party", "query_tasks",
                        "set_language", "help", "unclear"
                    ]
                },
                "kind": {
                    "type": "string",
                    "enum": ["sale", "expense", "payment_received", "payment_made"],
                    "description": "Only for record_transaction."
                },
                "amount": { "type": "number", "description": "Rupees, plain number." },
                "on_credit": { "type": "boolean", "description": "True for udhaar entries." },
                "party_name": { "type": "string", "description": "Customer or supplier." },
                "description": { "type": "string", "description": "Item or reason, a few words." },
                "assignee_name": { "type": "string", "description": "Employee for a task." },
                "title": { "type": "string", "description": "Short task description." },
                "due_date": { "type": "string", "description": "YYYY-MM-DD." },
                "task_hint": { "type": "string", "description": "What was finished." },
                "name": { "type": "string", "description": "New employee name." },
                "phone": { "type": "string", "description": "New employee phone, digits." },
                "period": { "type": "string", "enum": ["today", "week", "month"] },
                "language": {
                    "type": "string",
                    "enum": ["ur", "pa", "ps", "sd", "en"],
                    "description": "Only for set_language: the language to switch the bot to."
                },
                "reason": { "type": "string", "description": "Only for unclear, in English." }
            },
            "required": ["intent", "spoken_language"]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn client() -> ClaudeClient {
        ClaudeClient {
            http: Client::new(),
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
    fn request_forces_the_tool_call() {
        let body = client().request_body("ali ko paanch sau ka maal becha", &ctx());
        assert_eq!(body["tool_choice"]["type"], "tool");
        assert_eq!(body["tool_choice"]["name"], TOOL_NAME);
        assert_eq!(body["tools"][0]["name"], TOOL_NAME);
        assert_eq!(body["model"], "test-model");
        assert!(body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("paanch sau"));
    }

    #[test]
    fn finds_tool_input_in_response() {
        let response = json!({
            "content": [
                { "type": "text", "text": "ok" },
                { "type": "tool_use", "id": "t1", "name": TOOL_NAME,
                  "input": { "intent": "query_balance", "spoken_language": "ur" } }
            ]
        });
        let input = extract_tool_input(&response).unwrap();
        let parsed = ParsedCommand::from_value(input);
        assert_eq!(parsed.language.as_deref(), Some("ur"));
        assert_eq!(parsed.command, crate::nlp::Command::QueryBalance);
    }

    #[test]
    fn falls_back_to_text_json() {
        let response = json!({
            "content": [
                { "type": "text", "text": "```json\n{\"intent\":\"help\",\"spoken_language\":\"en\"}\n```" }
            ]
        });
        assert!(extract_tool_input(&response).is_none());
        let parsed = ParsedCommand::from_json_text(&extract_text(&response));
        assert_eq!(parsed.command, crate::nlp::Command::Help);
    }
}