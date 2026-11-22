use std::env;

use anyhow::{bail, Context, Result};

use crate::models::user::normalize_phone;

/// Which kind of API answers the "what does this voice note mean" question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NlpProvider {
    /// Any OpenAI-style /chat/completions endpoint: Groq, Gemini, OpenRouter, Ollama, OpenAI...
    OpenAiCompat,
    /// Anthropic's Messages API.
    Anthropic,
}

/// All runtime configuration, loaded once from environment variables (.env in dev).
/// Deliberately does not derive Debug so secrets never end up in logs.
#[derive(Clone)]
pub struct Config {
    pub bind_addr: String,
    pub database_url: String,

    pub whatsapp_verify_token: String,
    pub whatsapp_app_secret: String,
    pub whatsapp_access_token: String,
    pub whatsapp_phone_number_id: String,
    pub whatsapp_api_version: String,

    /// Base URL of an OpenAI-compatible speech API, without a trailing slash.
    pub stt_base_url: String,
    pub stt_api_key: String,
    pub stt_model: String,

    pub nlp_provider: NlpProvider,
    pub nlp_base_url: String,
    pub nlp_api_key: String,
    pub nlp_model: String,

    /// Normalized phone numbers (digits only, country code first) allowed to create a business.
    pub owner_allowlist: Vec<String>,
    /// Hour of day (0-23, Pakistan time) when the daily summary is sent.
    pub daily_summary_hour: u32,
}

fn required(key: &str) -> Result<String> {
    env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .with_context(|| format!("missing required environment variable {key}"))
}

fn optional(key: &str, default: &str) -> String {
    env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn trim_url(url: String) -> String {
    url.trim().trim_end_matches('/').to_string()
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let daily_summary_hour: u32 = optional("DAILY_SUMMARY_HOUR", "21")
            .parse()
            .context("DAILY_SUMMARY_HOUR must be a number between 0 and 23")?;
        if daily_summary_hour > 23 {
            bail!("DAILY_SUMMARY_HOUR must be between 0 and 23");
        }

        let owner_allowlist = optional("OWNER_ALLOWLIST", "")
            .split(',')
            .map(|s| normalize_phone(s.trim()))
            .filter(|s| !s.is_empty())
            .collect();

        let nlp_provider = match optional("NLP_PROVIDER", "openai_compat")
            .to_lowercase()
            .as_str()
        {
            "openai_compat" | "openai" | "groq" | "gemini" => NlpProvider::OpenAiCompat,
            "anthropic" | "claude" => NlpProvider::Anthropic,
            other => bail!("NLP_PROVIDER must be openai_compat or anthropic, got {other}"),
        };

        let default_nlp_model = match nlp_provider {
            NlpProvider::OpenAiCompat => "openai/gpt-oss-120b",
            NlpProvider::Anthropic => "claude-sonnet-5-5",
        };

        Ok(Self {
            bind_addr: optional("BIND_ADDR", "0.0.0.0:8090"),
            database_url: required("DATABASE_URL")?,

            whatsapp_verify_token: required("WHATSAPP_VERIFY_TOKEN")?,
            whatsapp_app_secret: required("WHATSAPP_APP_SECRET")?,
            whatsapp_access_token: required("WHATSAPP_ACCESS_TOKEN")?,
            whatsapp_phone_number_id: required("WHATSAPP_PHONE_NUMBER_ID")?,
            whatsapp_api_version: optional("WHATSAPP_API_VERSION", "v21.0"),

            stt_base_url: trim_url(optional("STT_BASE_URL", "https://api.groq.com/openai/v1")),
            stt_api_key: required("STT_API_KEY")?,
            stt_model: optional("STT_MODEL", "whisper-large-v3"),

            nlp_provider,
            nlp_base_url: trim_url(optional("NLP_BASE_URL", "https://api.groq.com/openai/v1")),
            nlp_api_key: required("NLP_API_KEY")?,
            nlp_model: optional("NLP_MODEL", default_nlp_model),

            owner_allowlist,
            daily_summary_hour,
        })
    }

    /// True if this phone number may register a brand-new business.
    pub fn is_allowed_owner(&self, phone: &str) -> bool {
        let normalized = normalize_phone(phone);
        self.owner_allowlist.iter().any(|p| *p == normalized)
    }
}