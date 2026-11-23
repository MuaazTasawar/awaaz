use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::nlp::intent::TxKind;

pub const STATUS_PENDING: &str = "pending";
pub const STATUS_CONFIRMED: &str = "confirmed";
pub const STATUS_CANCELLED: &str = "cancelled";

// The JSON payload is read and written as text, so sqlx does not need its optional json feature.
const COLUMNS: &str = "id, business_id, user_id, payload::text AS payload, summary, transcript, \
                       status, created_at, expires_at";

/// A fully resolved action waiting for the user's Yes. Names are already matched to real
/// customers and employees, so confirming it later needs no further guessing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Record {
        kind: TxKind,
        amount_paisa: i64,
        on_credit: bool,
        party_name: Option<String>,
        /// True when the customer or supplier did not exist yet and will be created.
        new_party: bool,
        description: Option<String>,
    },
    Assign {
        assignee_id: Uuid,
        assignee_name: String,
        title: String,
        due_date: Option<NaiveDate>,
    },
    Complete {
        task_id: Uuid,
        title: String,
        assignee_name: String,
    },
    AddEmployee {
        name: String,
        phone: String,
    },
}

/// What the bot is waiting for from this user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum PendingState {
    /// Waiting for Yes or No.
    Confirm { action: Action },
    /// Waiting for more details; the next message is added to the saved transcript.
    NeedInfo { attempts: u8 },
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PendingAction {
    pub id: Uuid,
    pub business_id: Uuid,
    pub user_id: Uuid,
    /// JSON text of a `PendingState`.
    pub payload: String,
    /// The question or read-back that was sent to the user.
    pub summary: String,
    /// What the user said that led here (grows when follow-up answers are added).
    pub transcript: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl PendingAction {
    pub fn state(&self) -> Option<PendingState> {
        serde_json::from_str(&self.payload).ok()
    }

    /// Save a new pending item. Anything still pending for this user is cancelled first,
    /// so a user only ever has one open question.
    pub async fn create(
        pool: &PgPool,
        business_id: Uuid,
        user_id: Uuid,
        state: &PendingState,
        summary: &str,
        transcript: &str,
    ) -> Result<PendingAction, sqlx::Error> {
        let payload =
            serde_json::to_string(state).map_err(|e| sqlx::Error::Protocol(e.to_string()))?;

        let mut tx = pool.begin().await?;
        sqlx::query(
            "UPDATE pending_actions SET status = 'cancelled'
             WHERE user_id = $1 AND status = 'pending'",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

        let sql = format!(
            "INSERT INTO pending_actions (business_id, user_id, payload, summary, transcript)
             VALUES ($1, $2, $3::jsonb, $4, $5)
             RETURNING {COLUMNS}"
        );
        let row = sqlx::query_as::<_, PendingAction>(&sql)
            .bind(business_id)
            .bind(user_id)
            .bind(payload)
            .bind(summary)
            .bind(transcript)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(row)
    }

    /// The user's open, not yet expired item (at most one).
    pub async fn find_active(
        pool: &PgPool,
        user_id: Uuid,
    ) -> Result<Option<PendingAction>, sqlx::Error> {
        let sql = format!(
            "SELECT {COLUMNS} FROM pending_actions
             WHERE user_id = $1 AND status = 'pending' AND expires_at > now()
             ORDER BY created_at DESC
             LIMIT 1"
        );
        sqlx::query_as::<_, PendingAction>(&sql)
            .bind(user_id)
            .fetch_optional(pool)
            .await
    }

    /// Move an open item to a new status. Returns true only for the one caller that won,
    /// so a double tap on Yes can never save an entry twice.
    pub async fn claim(pool: &PgPool, id: Uuid, new_status: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE pending_actions SET status = $2
             WHERE id = $1 AND status = 'pending' AND expires_at > now()",
        )
        .bind(id)
        .bind(new_status)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Put a confirmed item back to pending after a technical failure, so the user can try again.
    pub async fn release(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE pending_actions SET status = 'pending' WHERE id = $1 AND status = 'confirmed'")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Cancel everything this user has open.
    pub async fn cancel_all(pool: &PgPool, user_id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE pending_actions SET status = 'cancelled' WHERE user_id = $1 AND status = 'pending'")
            .bind(user_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Mark items past their expiry as expired. Returns how many were changed.
    pub async fn expire_stale(pool: &PgPool) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE pending_actions SET status = 'expired'
             WHERE status = 'pending' AND expires_at <= now()",
        )
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::test_support;

    #[test]
    fn states_survive_a_json_round_trip() {
        let confirm = PendingState::Confirm {
            action: Action::Record {
                kind: TxKind::Sale,
                amount_paisa: 250_000,
                on_credit: true,
                party_name: Some("Ali".to_string()),
                new_party: false,
                description: Some("cloth".to_string()),
            },
        };
        let json = serde_json::to_string(&confirm).unwrap();
        assert_eq!(serde_json::from_str::<PendingState>(&json).unwrap(), confirm);

        let assign = PendingState::Confirm {
            action: Action::Assign {
                assignee_id: Uuid::new_v4(),
                assignee_name: "Bilal".to_string(),
                title: "Count stock".to_string(),
                due_date: NaiveDate::from_ymd_opt(2026, 11, 23),
            },
        };
        let json = serde_json::to_string(&assign).unwrap();
        assert_eq!(serde_json::from_str::<PendingState>(&json).unwrap(), assign);

        let need = PendingState::NeedInfo { attempts: 2 };
        let json = serde_json::to_string(&need).unwrap();
        assert_eq!(serde_json::from_str::<PendingState>(&json).unwrap(), need);
    }

    #[tokio::test]
    async fn create_find_claim_and_release() {
        let Some(pool) = test_support::pool().await else {
            return;
        };
        let (business, owner) = test_support::business_with_owner(&pool).await;

        let first = PendingAction::create(
            &pool,
            business.id,
            owner.id,
            &PendingState::NeedInfo { attempts: 1 },
            "How much?",
            "ali ko maal becha",
        )
        .await
        .unwrap();
        assert_eq!(first.state(), Some(PendingState::NeedInfo { attempts: 1 }));

        // A second item replaces the first.
        let second = PendingAction::create(
            &pool,
            business.id,
            owner.id,
            &PendingState::NeedInfo { attempts: 2 },
            "Which one?",
            "more words",
        )
        .await
        .unwrap();
        let active = PendingAction::find_active(&pool, owner.id).await.unwrap().unwrap();
        assert_eq!(active.id, second.id);
        assert!(!PendingAction::claim(&pool, first.id, STATUS_CONFIRMED).await.unwrap());

        // Only one caller can claim it.
        assert!(PendingAction::claim(&pool, second.id, STATUS_CONFIRMED).await.unwrap());
        assert!(!PendingAction::claim(&pool, second.id, STATUS_CONFIRMED).await.unwrap());
        assert!(PendingAction::find_active(&pool, owner.id).await.unwrap().is_none());

        // After a technical failure it can be released and tried again.
        PendingAction::release(&pool, second.id).await.unwrap();
        assert!(PendingAction::find_active(&pool, owner.id).await.unwrap().is_some());

        PendingAction::cancel_all(&pool, owner.id).await.unwrap();
        assert!(PendingAction::find_active(&pool, owner.id).await.unwrap().is_none());

        test_support::cleanup(&pool, business.id).await;
    }
}