use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

pub const ROLE_OWNER: &str = "owner";
pub const ROLE_EMPLOYEE: &str = "employee";

const USER_COLUMNS: &str = "id, business_id, phone, name, role, language, is_active, created_at";

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct User {
    pub id: Uuid,
    pub business_id: Uuid,
    pub phone: String,
    pub name: String,
    pub role: String,
    pub language: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

impl User {
    pub fn is_owner(&self) -> bool {
        self.role == ROLE_OWNER
    }

    pub async fn create(
        pool: &PgPool,
        business_id: Uuid,
        phone: &str,
        name: &str,
        role: &str,
        language: &str,
    ) -> Result<User, sqlx::Error> {
        let sql = format!(
            "INSERT INTO users (business_id, phone, name, role, language)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING {USER_COLUMNS}"
        );
        sqlx::query_as::<_, User>(&sql)
            .bind(business_id)
            .bind(normalize_phone(phone))
            .bind(name)
            .bind(role)
            .bind(language)
            .fetch_one(pool)
            .await
    }

    /// Find an active user by WhatsApp number. This is how the bot knows who is talking.
    pub async fn find_active_by_phone(pool: &PgPool, phone: &str) -> Result<Option<User>, sqlx::Error> {
        let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE phone = $1 AND is_active = TRUE");
        sqlx::query_as::<_, User>(&sql)
            .bind(normalize_phone(phone))
            .fetch_optional(pool)
            .await
    }

    pub async fn find(pool: &PgPool, id: Uuid) -> Result<Option<User>, sqlx::Error> {
        let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE id = $1");
        sqlx::query_as::<_, User>(&sql).bind(id).fetch_optional(pool).await
    }

    pub async fn list_for_business(pool: &PgPool, business_id: Uuid) -> Result<Vec<User>, sqlx::Error> {
        let sql = format!(
            "SELECT {USER_COLUMNS} FROM users
             WHERE business_id = $1 AND is_active = TRUE
             ORDER BY created_at"
        );
        sqlx::query_as::<_, User>(&sql).bind(business_id).fetch_all(pool).await
    }

    /// Match a spoken name ("Ali", "علی") to employees of this business. Exact matches first.
    pub async fn find_in_business_by_name(
        pool: &PgPool,
        business_id: Uuid,
        spoken_name: &str,
    ) -> Result<Vec<User>, sqlx::Error> {
        let sql = format!(
            "SELECT {USER_COLUMNS} FROM users
             WHERE business_id = $1 AND is_active = TRUE AND name ILIKE '%' || $2 || '%'
             ORDER BY (lower(name) = lower($2)) DESC, name
             LIMIT 5"
        );
        sqlx::query_as::<_, User>(&sql)
            .bind(business_id)
            .bind(spoken_name.trim())
            .fetch_all(pool)
            .await
    }

    /// Every active owner across all businesses (used by the daily summary scheduler).
    pub async fn list_owners(pool: &PgPool) -> Result<Vec<User>, sqlx::Error> {
        let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE role = 'owner' AND is_active = TRUE");
        sqlx::query_as::<_, User>(&sql).fetch_all(pool).await
    }

    pub async fn set_language(pool: &PgPool, id: Uuid, language: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE users SET language = $2 WHERE id = $1")
            .bind(id)
            .bind(language)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn deactivate(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE users SET is_active = FALSE WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }
}

/// Turn any way of writing a Pakistani mobile number into digits with country code:
/// "0300-1234567", "+92 300 1234567", "3001234567" all become "923001234567".
pub fn normalize_phone(raw: &str) -> String {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if let Some(rest) = digits.strip_prefix("0092") {
        return format!("92{rest}");
    }
    if digits.starts_with('0') && digits.len() == 11 {
        return format!("92{}", &digits[1..]);
    }
    if digits.starts_with('3') && digits.len() == 10 {
        return format!("92{digits}");
    }
    digits
}

#[cfg(test)]
mod tests {
    use super::normalize_phone;

    #[test]
    fn normalizes_common_pakistani_formats() {
        assert_eq!(normalize_phone("0300-1234567"), "923001234567");
        assert_eq!(normalize_phone("+92 300 1234567"), "923001234567");
        assert_eq!(normalize_phone("00923001234567"), "923001234567");
        assert_eq!(normalize_phone("3001234567"), "923001234567");
        assert_eq!(normalize_phone("923001234567"), "923001234567");
    }
}