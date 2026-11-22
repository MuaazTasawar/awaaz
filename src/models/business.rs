use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// Language codes the bot can speak: Urdu, Punjabi, Pashto, Sindhi, English.
pub const SUPPORTED_LANGUAGES: [&str; 5] = ["ur", "pa", "ps", "sd", "en"];

pub fn is_supported_language(code: &str) -> bool {
    SUPPORTED_LANGUAGES.contains(&code)
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Business {
    pub id: Uuid,
    pub name: String,
    pub language: String,
    pub created_at: DateTime<Utc>,
}

impl Business {
    pub async fn create(pool: &PgPool, name: &str, language: &str) -> Result<Business, sqlx::Error> {
        sqlx::query_as::<_, Business>(
            "INSERT INTO businesses (name, language)
             VALUES ($1, $2)
             RETURNING id, name, language, created_at",
        )
        .bind(name)
        .bind(language)
        .fetch_one(pool)
        .await
    }

    pub async fn find(pool: &PgPool, id: Uuid) -> Result<Option<Business>, sqlx::Error> {
        sqlx::query_as::<_, Business>(
            "SELECT id, name, language, created_at FROM businesses WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(pool)
        .await
    }
}