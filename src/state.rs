use std::sync::Arc;

use sqlx::PgPool;

use crate::{config::Config, whatsapp::WhatsAppClient};

/// Shared application state handed to every request handler.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: PgPool,
    pub whatsapp: WhatsAppClient,
}

impl AppState {
    pub fn new(config: Config, db: PgPool) -> Self {
        let whatsapp = WhatsAppClient::new(&config);
        Self {
            config: Arc::new(config),
            db,
            whatsapp,
        }
    }
}