use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

const PARTY_COLUMNS: &str = "id, business_id, name, balance_paisa, created_at, updated_at";

/// A customer or supplier. `balance_paisa` > 0 means they owe us; < 0 means we owe them.
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Party {
    pub id: Uuid,
    pub business_id: Uuid,
    pub name: String,
    pub balance_paisa: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Result of looking a spoken name up among a business's parties.
#[derive(Debug)]
pub enum PartyMatch {
    NotFound,
    One(Party),
    /// Several parties fit (for example "Ali" when both "Ali Khan" and "Ali Raza" exist).
    /// The bot asks the user which one they mean.
    Many(Vec<Party>),
}

impl Party {
    /// Find the party by name or create it. Safe when two voice notes arrive at once.
    /// Pass a transaction connection (`&mut *tx`) to do this inside a transaction.
    pub async fn find_or_create<'e, E>(
        executor: E,
        business_id: Uuid,
        name: &str,
    ) -> Result<Party, sqlx::Error>
    where
        E: PgExecutor<'e>,
    {
        let sql = format!(
            "INSERT INTO parties (business_id, name, normalized_name)
             VALUES ($1, $2, $3)
             ON CONFLICT (business_id, normalized_name) DO UPDATE SET name = parties.name
             RETURNING {PARTY_COLUMNS}"
        );
        sqlx::query_as::<_, Party>(&sql)
            .bind(business_id)
            .bind(collapse_spaces(name))
            .bind(normalize_party_name(name))
            .fetch_one(executor)
            .await
    }

    /// Add `delta_paisa` (may be negative or zero) to the balance and return the updated party.
    pub async fn apply_delta<'e, E>(
        executor: E,
        id: Uuid,
        delta_paisa: i64,
    ) -> Result<Party, sqlx::Error>
    where
        E: PgExecutor<'e>,
    {
        let sql = format!(
            "UPDATE parties
             SET balance_paisa = balance_paisa + $2, updated_at = now()
             WHERE id = $1
             RETURNING {PARTY_COLUMNS}"
        );
        sqlx::query_as::<_, Party>(&sql)
            .bind(id)
            .bind(delta_paisa)
            .fetch_one(executor)
            .await
    }

    pub async fn find(pool: &PgPool, id: Uuid) -> Result<Option<Party>, sqlx::Error> {
        let sql = format!("SELECT {PARTY_COLUMNS} FROM parties WHERE id = $1");
        sqlx::query_as::<_, Party>(&sql)
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    /// Match a spoken name to this business's parties: exact (after normalizing) first,
    /// then "contains" in either direction ("Ali" matches "Ali Khan"; "Ali Khan Sahib" matches "Ali Khan").
    pub async fn resolve(
        pool: &PgPool,
        business_id: Uuid,
        spoken: &str,
    ) -> Result<PartyMatch, sqlx::Error> {
        let normalized = normalize_party_name(spoken);
        if normalized.chars().count() < 2 {
            return Ok(PartyMatch::NotFound);
        }

        let exact_sql = format!(
            "SELECT {PARTY_COLUMNS} FROM parties
             WHERE business_id = $1 AND normalized_name = $2"
        );
        if let Some(party) = sqlx::query_as::<_, Party>(&exact_sql)
            .bind(business_id)
            .bind(normalized.as_str())
            .fetch_optional(pool)
            .await?
        {
            return Ok(PartyMatch::One(party));
        }

        let fuzzy_sql = format!(
            "SELECT {PARTY_COLUMNS} FROM parties
             WHERE business_id = $1
               AND char_length(normalized_name) >= 2
               AND (strpos(normalized_name, $2) > 0 OR strpos($2, normalized_name) > 0)
             ORDER BY updated_at DESC
             LIMIT 5"
        );
        let mut rows = sqlx::query_as::<_, Party>(&fuzzy_sql)
            .bind(business_id)
            .bind(normalized.as_str())
            .fetch_all(pool)
            .await?;

        Ok(match rows.len() {
            0 => PartyMatch::NotFound,
            1 => PartyMatch::One(rows.remove(0)),
            _ => PartyMatch::Many(rows),
        })
    }

    /// Names of the most recently used parties. Handed to the language model so it can
    /// spell spoken names the way they are already stored.
    pub async fn list_recent_names(
        pool: &PgPool,
        business_id: Uuid,
        limit: i64,
    ) -> Result<Vec<String>, sqlx::Error> {
        sqlx::query_scalar::<_, String>(
            "SELECT name FROM parties
             WHERE business_id = $1
             ORDER BY updated_at DESC
             LIMIT $2",
        )
        .bind(business_id)
        .bind(limit)
        .fetch_all(pool)
        .await
    }
}

fn collapse_spaces(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Make two spellings of the same name compare equal: trim, collapse spaces, lower-case,
/// drop Arabic diacritics and tatweel, and unify Arabic and Urdu forms of yeh and kaf.
/// (Speech-to-text engines mix these forms freely.)
pub fn normalize_party_name(raw: &str) -> String {
    let mapped: String = raw
        .chars()
        .filter_map(|c| match c {
            // fathatan..sukun, other Arabic marks, superscript alef, tatweel
            '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{0640}' => None,
            // Arabic yeh and alef maksura become Urdu yeh
            '\u{064A}' | '\u{0649}' => Some('\u{06CC}'),
            // Arabic kaf becomes Urdu kaf
            '\u{0643}' => Some('\u{06A9}'),
            other => Some(other),
        })
        .collect();

    collapse_spaces(&mapped).to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::normalize_party_name;

    #[test]
    fn trims_collapses_and_lowercases() {
        assert_eq!(normalize_party_name("  Ali   KHAN "), "ali khan");
    }

    #[test]
    fn unifies_arabic_and_urdu_letter_variants() {
        // Arabic yeh U+064A versus Urdu yeh U+06CC
        assert_eq!(
            normalize_party_name("\u{0639}\u{0644}\u{064A}"),
            normalize_party_name("\u{0639}\u{0644}\u{06CC}")
        );
        // Arabic kaf U+0643 versus Urdu kaf U+06A9
        assert_eq!(
            normalize_party_name("\u{0643}\u{0631}\u{064A}\u{0645}"),
            normalize_party_name("\u{06A9}\u{0631}\u{06CC}\u{0645}")
        );
    }

    #[test]
    fn ignores_diacritics() {
        // ain + fatha + lam + yeh versus ain + lam + yeh
        assert_eq!(
            normalize_party_name("\u{0639}\u{064E}\u{0644}\u{06CC}"),
            normalize_party_name("\u{0639}\u{0644}\u{06CC}")
        );
    }
}