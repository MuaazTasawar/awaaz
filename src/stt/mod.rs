pub mod whisper;

pub use whisper::WhisperClient;

use async_trait::async_trait;

use crate::error::Result;

/// What the speech-to-text engine heard.
#[derive(Debug, Clone)]
pub struct Transcript {
    pub text: String,
    /// Language code detected by the engine (ur, pa, ps, sd, en...), when it reports one.
    pub language: Option<String>,
}

/// Anything that can turn a voice note into text. Whisper today; Google or Azure
/// speech services can be added later without touching the rest of the app.
#[async_trait]
pub trait SttProvider: Send + Sync {
    /// `language_hint` is an ISO code (ur, pa, ps, sd, en). Pass None to let the engine detect it.
    async fn transcribe(
        &self,
        audio: Vec<u8>,
        mime_type: &str,
        language_hint: Option<&str>,
    ) -> Result<Transcript>;
}