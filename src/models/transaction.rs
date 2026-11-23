use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use crate::nlp::intent::TxKind;

const TX_COLUMNS: &str = "id, business_id, party_id, user_id, kind, amount_paisa, on_credit, \
                          description, transcript, occurred_on, created_at";

/// One saved ledger entry.
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Transaction {
    pub id: Uuid,
    pub business_id: Uuid,
    pub party_id: Option<Uuid>,
    pub user_id: Uuid,
    pub kind: String,
    pub amount_paisa: i64,
    pub on_credit: bool,
    pub description: Option<String>,
    pub transcript: Option<String>,
    pub occurred_on: NaiveDate,
    pub created_at: DateTime<Utc>,
}

/// Everything needed to save an entry.
#[derive(Debug, Clone)]
pub struct NewTransaction {
    pub business_id: Uuid,
    pub party_id: Option<Uuid>,
    pub user_id: Uuid,
    pub kind: TxKind,
    pub amount_paisa: i64,
    pub on_credit: bool,
    pub description: Option<String>,
    pub transcript: Option<String>,
    pub occurred_on: NaiveDate,
}

impl Transaction {
    pub fn kind_enum(&self) -> Option<TxKind> {
        TxKind::from_db(&self.kind)
    }

    pub async fn insert<'e, E>(executor: E, new: &NewTransaction) -> Result<Transaction, sqlx::Error>
    where
        E: PgExecutor<'e>,
    {
        let sql = format!(
            "INSERT INTO transactions
                 (business_id, party_id, user_id, kind, amount_paisa, on_credit,
                  description, transcript, occurred_on)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING {TX_COLUMNS}"
        );
        sqlx::query_as::<_, Transaction>(&sql)
            .bind(new.business_id)
            .bind(new.party_id)
            .bind(new.user_id)
            .bind(new.kind.as_str())
            .bind(new.amount_paisa)
            .bind(new.on_credit)
            .bind(new.description.as_deref())
            .bind(new.transcript.as_deref())
            .bind(new.occurred_on)
            .fetch_one(executor)
            .await
    }

    /// Newest entries first for one customer or supplier.
    pub async fn list_for_party(
        pool: &PgPool,
        party_id: Uuid,
        limit: i64,
    ) -> Result<Vec<Transaction>, sqlx::Error> {
        let sql = format!(
            "SELECT {TX_COLUMNS} FROM transactions
             WHERE party_id = $1
             ORDER BY occurred_on DESC, created_at DESC
             LIMIT $2"
        );
        sqlx::query_as::<_, Transaction>(&sql)
            .bind(party_id)
            .bind(limit)
            .fetch_all(pool)
            .await
    }
}

/// How much cash enters (+) or leaves (-) the till because of this entry.
pub fn cash_delta_paisa(kind: TxKind, on_credit: bool, amount_paisa: i64) -> i64 {
    kind.cash_sign(on_credit) * amount_paisa
}

/// How the entry changes what the party owes us (+) or what we owe them (-).
pub fn party_delta_paisa(kind: TxKind, on_credit: bool, amount_paisa: i64) -> i64 {
    kind.party_sign(on_credit) * amount_paisa
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_sale_moves_no_cash_but_party_owes_us() {
        assert_eq!(cash_delta_paisa(TxKind::Sale, true, 250_000), 0);
        assert_eq!(party_delta_paisa(TxKind::Sale, true, 250_000), 250_000);
    }

    #[test]
    fn customer_payment_adds_cash_and_reduces_debt() {
        assert_eq!(cash_delta_paisa(TxKind::PaymentReceived, false, 100_000), 100_000);
        assert_eq!(party_delta_paisa(TxKind::PaymentReceived, false, 100_000), -100_000);
    }

    #[test]
    fn credit_purchase_means_we_owe_the_supplier() {
        assert_eq!(cash_delta_paisa(TxKind::Expense, true, 500_000), 0);
        assert_eq!(party_delta_paisa(TxKind::Expense, true, 500_000), -500_000);
    }

    #[test]
    fn cash_expense_reduces_cash_only() {
        assert_eq!(cash_delta_paisa(TxKind::Expense, false, 30_000), -30_000);
        assert_eq!(party_delta_paisa(TxKind::Expense, false, 30_000), 0);
    }
}