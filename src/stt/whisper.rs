use std::time::Duration;

use async_trait::async_trait;
use reqwest::{
    multipart::{Form, Part},
    Client,
};
use serde::Deserialize;

use crate::{
    config::Config,
    error::{AppError, Result},
    stt::{SttProvider, Transcript},
};

/// Anything shorter than this is not a real voice note (a one-second WhatsApp note is several KB).
const MIN_AUDIO_BYTES: usize = 500;

/// Free tiers answer 429 when you go over the per-minute limit; wait and try again a couple of times.
const MAX_ATTEMPTS: u64 = 3;

/// Languages Whisper accepts as a hint for the languages Awaaz supports.
const WHISPER_LANGS: [&str; 5] = ["ur", "pa", "ps", "sd", "en"];

/// A short Urdu sentence that nudges Whisper to write Urdu in Urdu script and to expect
/// business vocabulary (sold, bought, credit, received, thousand, lakh).
const URDU_PROMPT: &str =
    "کاروبار کا حساب: بیچا، خریدا، ادھار، وصول ہوئے، ہزار روپے، لاکھ روپے، کل تک کام مکمل کرنا۔";

/// Client for any OpenAI-compatible `/audio/transcriptions` endpoint
/// (Groq, OpenAI, or a local Whisper server).
pub struct WhisperClient {
    http: Client,
    url: String,
    api_key: String,
    model: String,
}

#[derive(Debug, Deserialize)]
struct WhisperResponse {
    #[serde(default)]
    text: String,
    #[serde(default)]
    language: Option<String>,
}

impl WhisperClient {
    pub fn new(config: &Config) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(90))
            .build()
            .expect("failed to build HTTP client");

        Self {
            http,
            url: format!("{}/audio/transcriptions", config.stt_base_url),
            api_key: config.stt_api_key.clone(),
            model: config.stt_model.clone(),
        }
    }

    /// The upload form is rebuilt for every attempt because sending consumes it.
    fn build_form(&self, audio: &[u8], mime_type: &str, language_hint: Option<&str>) -> Result<Form> {
        let (ext, mime) = audio_format(mime_type);
        let part = Part::bytes(audio.to_vec())
            .file_name(format!("voice.{ext}"))
            .mime_str(mime)
            .map_err(|e| AppError::Stt(e.to_string()))?;

        let mut form = Form::new()
            .part("file", part)
            .text("model", self.model.clone())
            .text("response_format", "verbose_json")
            .text("temperature", "0");

        if let Some(code) = language_hint {
            if WHISPER_LANGS.contains(&code) {
                form = form.text("language", code.to_string());
                if code == "ur" {
                    form = form.text("prompt", URDU_PROMPT);
                }
            }
        }

        Ok(form)
    }
}

#[async_trait]
impl SttProvider for WhisperClient {
    async fn transcribe(
        &self,
        audio: Vec<u8>,
        mime_type: &str,
        language_hint: Option<&str>,
    ) -> Result<Transcript> {
        if audio.len() < MIN_AUDIO_BYTES {
            return Err(AppError::Stt("audio too short".to_string()));
        }

        let mut last_error = AppError::Stt("no attempt made".to_string());

        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2 * attempt)).await;
            }

            let form = self.build_form(&audio, mime_type, language_hint)?;
            let resp = match self
                .http
                .post(&self.url)
                .bearer_auth(&self.api_key)
                .multipart(form)
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
                let parsed: WhisperResponse = resp.json().await?;
                let text = parsed.text.trim().to_string();
                if text.is_empty() {
                    return Err(AppError::Stt("no speech detected".to_string()));
                }
                return Ok(Transcript {
                    text,
                    language: parsed
                        .language
                        .as_deref()
                        .and_then(language_code)
                        .map(str::to_string),
                });
            }

            let body = resp.text().await.unwrap_or_default();
            let short: String = body.chars().take(300).collect();
            last_error = AppError::Stt(format!("{status}: {short}"));

            let retryable = status.as_u16() == 429 || status.is_server_error();
            if !retryable {
                break;
            }
        }

        Err(last_error)
    }
}

/// Map WhatsApp's mime type to a file extension and a clean mime type Whisper accepts.
/// WhatsApp voice notes are normally "audio/ogg; codecs=opus".
fn audio_format(mime_type: &str) -> (&'static str, &'static str) {
    let base = mime_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();

    match base.as_str() {
        "audio/ogg" | "audio/opus" => ("ogg", "audio/ogg"),
        "audio/mpeg" | "audio/mp3" => ("mp3", "audio/mpeg"),
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" | "audio/aac" => ("m4a", "audio/mp4"),
        "audio/wav" | "audio/x-wav" | "audio/wave" => ("wav", "audio/wav"),
        "audio/webm" => ("webm", "audio/webm"),
        "audio/flac" => ("flac", "audio/flac"),
        _ => ("ogg", "audio/ogg"),
    }
}

/// Whisper reports the detected language by English name ("urdu"); convert to our codes.
fn language_code(name: &str) -> Option<&'static str> {
    match name.trim().to_lowercase().as_str() {
        "urdu" | "ur" => Some("ur"),
        "punjabi" | "panjabi" | "pa" => Some("pa"),
        "pashto" | "pushto" | "ps" => Some("ps"),
        "sindhi" | "sd" => Some("sd"),
        "english" | "en" => Some("en"),
        "hindi" | "hi" => Some("hi"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_extension_from_mime() {
        assert_eq!(audio_format("audio/ogg; codecs=opus"), ("ogg", "audio/ogg"));
        assert_eq!(audio_format("audio/mpeg"), ("mp3", "audio/mpeg"));
        assert_eq!(audio_format("audio/aac"), ("m4a", "audio/mp4"));
        assert_eq!(audio_format("something/else"), ("ogg", "audio/ogg"));
    }

    #[test]
    fn maps_language_names_to_codes() {
        assert_eq!(language_code("Urdu"), Some("ur"));
        assert_eq!(language_code("punjabi"), Some("pa"));
        assert_eq!(language_code("Pashto"), Some("ps"));
        assert_eq!(language_code("sindhi"), Some("sd"));
        assert_eq!(language_code("klingon"), None);
    }
}