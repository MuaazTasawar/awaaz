use std::sync::Arc;

use sqlx::PgPool;

use crate::{
    config::Config,
    nlp::{ClaudeClient, IntentParser},
    stt::{SttProvider, WhisperClient},
    whatsapp::WhatsAppClient,
};

/// Shared application state handed to every request handler.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: PgPool,
    pub whatsapp: WhatsAppClient,
    pub stt: Arc<dyn SttProvider>,
    pub parser: Arc<dyn IntentParser>,
}

impl AppState {
    pub fn new(config: Config, db: PgPool) -> Self {
        let whatsapp = WhatsAppClient::new(&config);
        let stt: Arc<dyn SttProvider> = Arc::new(WhisperClient::new(&config));
        let parser: Arc<dyn IntentParser> = Arc::new(ClaudeClient::new(&config));

        Self {
            config: Arc::new(config),
            db,
            whatsapp,
            stt,
            parser,
        }
    }
}