use chrono::{Datelike, NaiveDate};
use serde::Serialize;
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use crate::{
    error::{AppError, Result},
    models::{
        party::{Party, PartyMatch},
        transaction::{party_delta_paisa, NewTransaction, Transaction},
        user::User,
    },
    nlp::{intent::MAX_AMOUNT_PKR, Period, TxKind},
};

const MAX_DESCRIPTION_CHARS: usize = 200;
const MAX_TRANSCRIPT_CHARS: usize = 2000;

/// What the user asked to record, already converted to integer paisa.
#[derive(Debug, Clone)]
pub struct NewEntry {
    pub kind: TxKind,
    pub amount_paisa: i64,
    pub on_credit: bool,
    pub party_name: Option<String>,
    pub description: Option<String>,
    pub transcript: Option<String>,
    /// Pakistan business date. Use `nlp::pakistan_today()` unless the user names another day.
    pub occurred_on: NaiveDate,
}

/// What came out of saving an entry, for the confirmation message.
#[derive(Debug, Clone)]
pub struct RecordedTransaction {
    pub transaction: Transaction,
    /// The customer or supplier after their balance was updated, if the entry named one.
    pub party: Option<Party>,
    /// Cash in hand after this entry.
    pub cash_balance_paisa: i64,
}

/// Totals for a date range plus the current position.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Summary {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub entries: i64,
    /// All sales, cash and credit.
    pub sales_paisa: i64,
    pub credit_sales_paisa: i64,
    /// All expenses, cash and credit.
    pub expenses_paisa: i64,
    pub credit_expenses_paisa: i64,
    pub received_paisa: i64,
    pub paid_paisa: i64,
    /// Cash in hand right now (all time, not just this range).
    pub cash_balance_paisa: i64,
    /// Total customers owe us right now.
    pub receivable_paisa: i64,
    /// Total we owe suppliers right now.
    pub payable_paisa: i64,
}

impl Summary {
    /// Cash that came in during the range: cash sales plus payments received.
    pub fn cash_in_paisa(&self) -> i64 {
        self.sales_paisa - self.credit_sales_paisa + self.received_paisa
    }

    /// Cash that went out during the range: cash expenses plus payments made.
    pub fn cash_out_paisa(&self) -> i64 {
        self.expenses_paisa - self.credit_expenses_paisa + self.paid_paisa
    }

    /// Sales minus expenses (credit entries count when they happen, not when they are paid).
    pub fn profit_paisa(&self) -> i64 {
        self.sales_paisa - self.expenses_paisa
    }
}

/// One customer or supplier with their latest entries.
#[derive(Debug, Clone)]
pub struct PartyStatement {
    pub party: Party,
    pub entries: Vec<Transaction>,
}

/// Payments never create new debt, so the credit flag only counts for sales and purchases.
pub fn effective_on_credit(kind: TxKind, on_credit: bool) -> bool {
    on_credit && matches!(kind, TxKind::Sale | TxKind::Expense)
}

fn validate_entry(entry: &NewEntry) -> Result<()> {
    let max_paisa = (MAX_AMOUNT_PKR * 100.0) as i64;
    if entry.amount_paisa <= 0 || entry.amount_paisa > max_paisa {
        return Err(AppError::BadRequest("invalid amount".to_string()));
    }

    if let Some(name) = entry.party_name.as_deref() {
        let len = name.trim().chars().count();
        if !(2..=80).contains(&len) {
            return Err(AppError::BadRequest(
                "name must be 2 to 80 characters".to_string(),
            ));
        }
    }

    let on_credit = effective_on_credit(entry.kind, entry.on_credit);
    let needs_party =
        on_credit || matches!(entry.kind, TxKind::PaymentReceived | TxKind::PaymentMade);
    let has_party = entry
        .party_name
        .as_deref()
        .map(|n| !n.trim().is_empty())
        .unwrap_or(false);
    if needs_party && !has_party {
        return Err(AppError::BadRequest(
            "a customer or supplier name is required".to_string(),
        ));
    }

    Ok(())
}

fn clip(value: Option<String>, max_chars: usize) -> Option<String> {
    value
        .map(|v| v.trim().chars().take(max_chars).collect::<String>())
        .filter(|v| !v.is_empty())
}

/// Save one entry, creating the customer or supplier if needed and updating their balance.
/// Everything happens in one database transaction: either all of it is saved or none of it.
pub async fn record(pool: &PgPool, user: &User, entry: NewEntry) -> Result<RecordedTransaction> {
    validate_entry(&entry)?;
    let on_credit = effective_on_credit(entry.kind, entry.on_credit);

    let mut tx = pool.begin().await?;

    let party_id = match entry.party_name.as_deref() {
        Some(name) => Some(Party::find_or_create(&mut *tx, user.business_id, name).await?.id),
        None => None,
    };

    let row = Transaction::insert(
        &mut *tx,
        &NewTransaction {
            business_id: user.business_id,
            party_id,
            user_id: user.id,
            kind: entry.kind,
            amount_paisa: entry.amount_paisa,
            on_credit,
            description: clip(entry.description, MAX_DESCRIPTION_CHARS),
            transcript: clip(entry.transcript, MAX_TRANSCRIPT_CHARS),
            occurred_on: entry.occurred_on,
        },
    )
    .await?;

    let party = match party_id {
        Some(id) => {
            let delta = party_delta_paisa(entry.kind, on_credit, entry.amount_paisa);
            Some(Party::apply_delta(&mut *tx, id, delta).await?)
        }
        None => None,
    };

    let cash_balance_paisa = cash_balance(&mut *tx, user.business_id).await?;
    tx.commit().await?;

    Ok(RecordedTransaction {
        transaction: row,
        party,
        cash_balance_paisa,
    })
}

/// Cash in hand: cash sales and payments received, minus cash expenses and payments made.
/// Credit entries move no cash until the customer or supplier is paid.
pub async fn cash_balance<'e, E>(executor: E, business_id: Uuid) -> Result<i64>
where
    E: PgExecutor<'e>,
{
    let balance: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(CASE
                    WHEN kind = 'sale'             AND NOT on_credit THEN  amount_paisa
                    WHEN kind = 'expense'          AND NOT on_credit THEN -amount_paisa
                    WHEN kind = 'payment_received'                   THEN  amount_paisa
                    WHEN kind = 'payment_made'                       THEN -amount_paisa
                    ELSE 0
                END), 0)::BIGINT
         FROM transactions
         WHERE business_id = $1",
    )
    .bind(business_id)
    .fetch_one(executor)
    .await?;
    Ok(balance)
}

/// Totals for `from..=to` (Pakistan dates) plus today's cash, receivable and payable position.
pub async fn summary(
    pool: &PgPool,
    business_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Summary> {
    let row: (i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
             COUNT(*)::BIGINT,
             COALESCE(SUM(amount_paisa) FILTER (WHERE kind = 'sale'), 0)::BIGINT,
             COALESCE(SUM(amount_paisa) FILTER (WHERE kind = 'sale' AND on_credit), 0)::BIGINT,
             COALESCE(SUM(amount_paisa) FILTER (WHERE kind = 'expense'), 0)::BIGINT,
             COALESCE(SUM(amount_paisa) FILTER (WHERE kind = 'expense' AND on_credit), 0)::BIGINT,
             COALESCE(SUM(amount_paisa) FILTER (WHERE kind = 'payment_received'), 0)::BIGINT,
             COALESCE(SUM(amount_paisa) FILTER (WHERE kind = 'payment_made'), 0)::BIGINT
         FROM transactions
         WHERE business_id = $1 AND occurred_on BETWEEN $2 AND $3",
    )
    .bind(business_id)
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await?;

    let (receivable_paisa, payable_paisa) = party_totals(pool, business_id).await?;
    let cash_balance_paisa = cash_balance(pool, business_id).await?;

    Ok(Summary {
        from,
        to,
        entries: row.0,
        sales_paisa: row.1,
        credit_sales_paisa: row.2,
        expenses_paisa: row.3,
        credit_expenses_paisa: row.4,
        received_paisa: row.5,
        paid_paisa: row.6,
        cash_balance_paisa,
        receivable_paisa,
        payable_paisa,
    })
}

/// (total customers owe us, total we owe suppliers), both as positive numbers.
pub async fn party_totals(pool: &PgPool, business_id: Uuid) -> Result<(i64, i64)> {
    let totals: (i64, i64) = sqlx::query_as(
        "SELECT
             COALESCE(SUM(balance_paisa) FILTER (WHERE balance_paisa > 0), 0)::BIGINT,
             COALESCE(-SUM(balance_paisa) FILTER (WHERE balance_paisa < 0), 0)::BIGINT
         FROM parties
         WHERE business_id = $1",
    )
    .bind(business_id)
    .fetch_one(pool)
    .await?;
    Ok(totals)
}

/// Match a spoken name to a customer or supplier of this business.
pub async fn find_party(pool: &PgPool, business_id: Uuid, spoken: &str) -> Result<PartyMatch> {
    Ok(Party::resolve(pool, business_id, spoken).await?)
}

/// A party plus their newest `limit` entries.
pub async fn party_statement(pool: &PgPool, party: Party, limit: i64) -> Result<PartyStatement> {
    let entries = Transaction::list_for_party(pool, party.id, limit).await?;
    Ok(PartyStatement { party, entries })
}

/// Names to hand to the language model so spoken names are matched to stored spellings.
pub async fn known_party_names(pool: &PgPool, business_id: Uuid) -> Result<Vec<String>> {
    Ok(Party::list_recent_names(pool, business_id, 100).await?)
}

/// The date range a spoken period means: today; the last 7 days; or the 1st of this month up to today.
pub fn period_range(period: Period, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    match period {
        Period::Today => (today, today),
        Period::Week => (today - chrono::Duration::days(6), today),
        Period::Month => (today.with_day(1).unwrap_or(today), today),
    }
}

/// "Rs 2,500", "Rs 1,500.75", "-Rs 50". Whole rupees drop the decimals.
pub fn format_rupees(paisa: i64) -> String {
    let sign = if paisa < 0 { "-" } else { "" };
    let abs = paisa.unsigned_abs();
    let rupees = abs / 100;
    let frac = abs % 100;

    let digits = rupees.to_string();
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }

    if frac == 0 {
        format!("{sign}Rs {grouped}")
    } else {
        format!("{sign}Rs {grouped}.{frac:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db,
        models::{business::Business, user::ROLE_OWNER},
    };

    // ---------- pure logic, no database needed ----------

    #[test]
    fn formats_rupees() {
        assert_eq!(format_rupees(0), "Rs 0");
        assert_eq!(format_rupees(250_000), "Rs 2,500");
        assert_eq!(format_rupees(150_075), "Rs 1,500.75");
        assert_eq!(format_rupees(100_000_000), "Rs 1,000,000");
        assert_eq!(format_rupees(-5_000), "-Rs 50");
        assert_eq!(format_rupees(99), "Rs 0.99");
    }

    #[test]
    fn period_ranges() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        assert_eq!(period_range(Period::Today, today), (today, today));
        assert_eq!(
            period_range(Period::Week, today),
            (NaiveDate::from_ymd_opt(2026, 11, 16).unwrap(), today)
        );
        assert_eq!(
            period_range(Period::Month, today),
            (NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(), today)
        );
    }

    #[test]
    fn payments_are_never_credit() {
        assert!(effective_on_credit(TxKind::Sale, true));
        assert!(effective_on_credit(TxKind::Expense, true));
        assert!(!effective_on_credit(TxKind::PaymentReceived, true));
        assert!(!effective_on_credit(TxKind::PaymentMade, true));
        assert!(!effective_on_credit(TxKind::Sale, false));
    }

    fn entry(kind: TxKind, paisa: i64, on_credit: bool, party: Option<&str>) -> NewEntry {
        NewEntry {
            kind,
            amount_paisa: paisa,
            on_credit,
            party_name: party.map(str::to_string),
            description: None,
            transcript: None,
            occurred_on: NaiveDate::from_ymd_opt(2026, 11, 22).unwrap(),
        }
    }

    #[test]
    fn validation_rejects_bad_entries() {
        assert!(validate_entry(&entry(TxKind::Sale, 0, false, None)).is_err());
        assert!(validate_entry(&entry(TxKind::Sale, -5, false, None)).is_err());
        assert!(validate_entry(&entry(TxKind::Sale, i64::MAX, false, None)).is_err());
        // credit sale and payments need a name
        assert!(validate_entry(&entry(TxKind::Sale, 1000, true, None)).is_err());
        assert!(validate_entry(&entry(TxKind::PaymentReceived, 1000, false, None)).is_err());
        // one-letter names are rejected
        assert!(validate_entry(&entry(TxKind::Sale, 1000, true, Some("A"))).is_err());
    }

    #[test]
    fn validation_accepts_good_entries() {
        assert!(validate_entry(&entry(TxKind::Sale, 1000, false, None)).is_ok());
        assert!(validate_entry(&entry(TxKind::Sale, 1000, true, Some("Ali"))).is_ok());
        assert!(validate_entry(&entry(TxKind::PaymentMade, 1000, false, Some("Supplier"))).is_ok());
        // a credit flag on a payment does not require anything extra beyond the name it already needs
        assert!(validate_entry(&entry(TxKind::PaymentReceived, 1000, true, Some("Ali"))).is_ok());
    }

    // ---------- database tests ----------
    // They use the DATABASE_URL from .env and skip themselves (with a message) when the
    // database is not reachable. Each test creates its own business and deletes it at the end.

    async fn test_pool() -> Option<PgPool> {
        dotenvy::dotenv().ok();
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!("skipping database test: DATABASE_URL is not set");
                return None;
            }
        };
        let pool = match db::connect(&url).await {
            Ok(pool) => pool,
            Err(e) => {
                eprintln!("skipping database test: cannot connect ({e})");
                return None;
            }
        };
        if let Err(e) = db::run_migrations(&pool).await {
            eprintln!("skipping database test: migrations failed ({e})");
            return None;
        }
        Some(pool)
    }

    async fn setup(pool: &PgPool) -> (Business, User) {
        let business = Business::create(pool, "Test Shop", "ur").await.unwrap();
        let phone = format!("92{:010}", Uuid::new_v4().as_u128() % 10_000_000_000);
        let user = User::create(pool, business.id, &phone, "Owner", ROLE_OWNER, "ur")
            .await
            .unwrap();
        (business, user)
    }

    async fn cleanup(pool: &PgPool, business_id: Uuid) {
        sqlx::query("DELETE FROM businesses WHERE id = $1")
            .bind(business_id)
            .execute(pool)
            .await
            .ok();
    }

    #[tokio::test]
    async fn balances_follow_a_day_of_trading() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let (business, user) = setup(&pool).await;

        // Rs 2,500 of cloth sold to Ali on credit: no cash, Ali owes us.
        let r = record(&pool, &user, entry(TxKind::Sale, 250_000, true, Some("Ali")))
            .await
            .unwrap();
        assert_eq!(r.party.as_ref().unwrap().balance_paisa, 250_000);
        assert_eq!(r.cash_balance_paisa, 0);

        // Rs 1,000 cash sale, no customer named.
        let r = record(&pool, &user, entry(TxKind::Sale, 100_000, false, None))
            .await
            .unwrap();
        assert!(r.party.is_none());
        assert_eq!(r.cash_balance_paisa, 100_000);

        // Ali pays Rs 1,000 (spoken with different capitalization): same customer, less debt.
        let r = record(
            &pool,
            &user,
            entry(TxKind::PaymentReceived, 100_000, false, Some("ali")),
        )
        .await
        .unwrap();
        assert_eq!(r.party.as_ref().unwrap().balance_paisa, 150_000);
        assert_eq!(r.cash_balance_paisa, 200_000);

        // Rs 300 cash expense.
        let r = record(&pool, &user, entry(TxKind::Expense, 30_000, false, None))
            .await
            .unwrap();
        assert_eq!(r.cash_balance_paisa, 170_000);

        // Rs 5,000 of stock bought from a supplier on credit: no cash, we owe them.
        let r = record(
            &pool,
            &user,
            entry(TxKind::Expense, 500_000, true, Some("Supplier Co")),
        )
        .await
        .unwrap();
        assert_eq!(r.party.as_ref().unwrap().balance_paisa, -500_000);
        assert_eq!(r.cash_balance_paisa, 170_000);

        let day = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        let s = summary(&pool, business.id, day, day).await.unwrap();
        assert_eq!(s.entries, 5);
        assert_eq!(s.sales_paisa, 350_000);
        assert_eq!(s.credit_sales_paisa, 250_000);
        assert_eq!(s.expenses_paisa, 530_000);
        assert_eq!(s.credit_expenses_paisa, 500_000);
        assert_eq!(s.received_paisa, 100_000);
        assert_eq!(s.paid_paisa, 0);
        assert_eq!(s.cash_in_paisa(), 200_000);
        assert_eq!(s.cash_out_paisa(), 30_000);
        assert_eq!(s.profit_paisa(), -180_000);
        assert_eq!(s.cash_balance_paisa, 170_000);
        assert_eq!(s.receivable_paisa, 150_000);
        assert_eq!(s.payable_paisa, 500_000);

        // An unrelated day is empty, but the current position is still reported.
        let other = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let s = summary(&pool, business.id, other, other).await.unwrap();
        assert_eq!(s.entries, 0);
        assert_eq!(s.cash_balance_paisa, 170_000);

        cleanup(&pool, business.id).await;
    }

    #[tokio::test]
    async fn spoken_names_resolve_to_the_right_party() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let (business, user) = setup(&pool).await;

        record(&pool, &user, entry(TxKind::Sale, 1_000, true, Some("Ali Khan")))
            .await
            .unwrap();
        record(&pool, &user, entry(TxKind::Sale, 2_000, true, Some("Ali Raza")))
            .await
            .unwrap();

        // Full name: exactly one.
        let found = find_party(&pool, business.id, "ali khan").await.unwrap();
        assert!(matches!(found, PartyMatch::One(ref p) if p.name == "Ali Khan"));

        // First name only: two fit, so the bot must ask which one.
        let found = find_party(&pool, business.id, "Ali").await.unwrap();
        assert!(matches!(found, PartyMatch::Many(ref v) if v.len() == 2));

        // A longer spoken form still finds the stored name.
        let found = find_party(&pool, business.id, "Ali Raza Sahib").await.unwrap();
        assert!(matches!(found, PartyMatch::One(ref p) if p.name == "Ali Raza"));

        let found = find_party(&pool, business.id, "Zubair").await.unwrap();
        assert!(matches!(found, PartyMatch::NotFound));

        // The party statement lists newest entries first.
        let ali = match find_party(&pool, business.id, "ali khan").await.unwrap() {
            PartyMatch::One(p) => p,
            other => panic!("expected one party, got {other:?}"),
        };
        let statement = party_statement(&pool, ali, 10).await.unwrap();
        assert_eq!(statement.entries.len(), 1);
        assert_eq!(statement.party.balance_paisa, 1_000);

        let names = known_party_names(&pool, business.id).await.unwrap();
        assert_eq!(names.len(), 2);

        cleanup(&pool, business.id).await;
    }
}