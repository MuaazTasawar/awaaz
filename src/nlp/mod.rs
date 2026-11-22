pub mod claude;
pub mod intent;
pub mod openai_compat;
pub mod prompt;

pub use claude::ClaudeClient;
pub use intent::{rupees_to_paisa, Command, Missing, ParsedCommand, Period, TxKind};
pub use openai_compat::OpenAiCompatClient;

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};

use crate::error::Result;

/// Extra knowledge handed to the language model so it can resolve "kal", match spoken names
/// to real employees and customers, and spell known names the way they are stored.
#[derive(Debug, Clone)]
pub struct ParseContext {
    pub today: NaiveDate,
    pub employee_names: Vec<String>,
    pub party_names: Vec<String>,
}

/// Turns a transcript into one structured command.
#[async_trait]
pub trait IntentParser: Send + Sync {
    async fn parse(&self, transcript: &str, ctx: &ParseContext) -> Result<ParsedCommand>;
}

/// Current time in Pakistan (UTC+5, no daylight saving).
pub fn pakistan_now() -> DateTime<FixedOffset> {
    let tz = FixedOffset::east_opt(5 * 3600).expect("valid fixed offset");
    Utc::now().with_timezone(&tz)
}

/// Today's date in Pakistan. Business days roll over at local midnight, not UTC midnight.
pub fn pakistan_today() -> NaiveDate {
    pakistan_now().date_naive()
}