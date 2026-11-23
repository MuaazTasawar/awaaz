use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::Result,
    models::party::Party,
    nlp::Period,
    services::ledger::{self, format_rupees, Summary},
};

const TOP_PARTIES: i64 = 3;

/// Task counts for the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStats {
    /// Open right now.
    pub open: i64,
    /// Open and past their due date.
    pub overdue: i64,
    /// Finished during the report range.
    pub done_in_range: i64,
    /// Created during the report range.
    pub created_in_range: i64,
}

/// Everything the "how was my day" answer and the daily summary need.
#[derive(Debug, Clone)]
pub struct Digest {
    pub summary: Summary,
    pub tasks: TaskStats,
    /// Customers who owe the most, biggest first.
    pub top_receivables: Vec<Party>,
    /// Suppliers owed the most, biggest first (their balance is negative).
    pub top_payables: Vec<Party>,
}

/// Task counts for a business. `today` decides which open tasks are overdue; the range
/// decides what counts as created or finished "in this report".
pub async fn task_stats(
    pool: &PgPool,
    business_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
    today: NaiveDate,
) -> Result<TaskStats> {
    let row: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
             COUNT(*) FILTER (WHERE status = 'open')::BIGINT,
             COUNT(*) FILTER (WHERE status = 'open'
                                AND due_date IS NOT NULL AND due_date < $2)::BIGINT,
             COUNT(*) FILTER (WHERE status = 'done'
                                AND (completed_at AT TIME ZONE 'Asia/Karachi')::date
                                    BETWEEN $3 AND $4)::BIGINT,
             COUNT(*) FILTER (WHERE (created_at AT TIME ZONE 'Asia/Karachi')::date
                                    BETWEEN $3 AND $4)::BIGINT
         FROM tasks
         WHERE business_id = $1",
    )
    .bind(business_id)
    .bind(today)
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await?;

    Ok(TaskStats {
        open: row.0,
        overdue: row.1,
        done_in_range: row.2,
        created_in_range: row.3,
    })
}

async fn top_parties(
    pool: &PgPool,
    business_id: Uuid,
    receivable: bool,
    limit: i64,
) -> Result<Vec<Party>> {
    let sql = if receivable {
        "SELECT id, business_id, name, balance_paisa, created_at, updated_at
         FROM parties
         WHERE business_id = $1 AND balance_paisa > 0
         ORDER BY balance_paisa DESC
         LIMIT $2"
    } else {
        "SELECT id, business_id, name, balance_paisa, created_at, updated_at
         FROM parties
         WHERE business_id = $1 AND balance_paisa < 0
         ORDER BY balance_paisa ASC
         LIMIT $2"
    };

    Ok(sqlx::query_as::<_, Party>(sql)
        .bind(business_id)
        .bind(limit)
        .fetch_all(pool)
        .await?)
}

/// Build the report for a period ending today (Pakistan date).
pub async fn build_digest(
    pool: &PgPool,
    business_id: Uuid,
    period: Period,
    today: NaiveDate,
) -> Result<Digest> {
    let (from, to) = ledger::period_range(period, today);
    let summary = ledger::summary(pool, business_id, from, to).await?;
    let tasks = task_stats(pool, business_id, from, to, today).await?;
    let top_receivables = top_parties(pool, business_id, true, TOP_PARTIES).await?;
    let top_payables = top_parties(pool, business_id, false, TOP_PARTIES).await?;

    Ok(Digest {
        summary,
        tasks,
        top_receivables,
        top_payables,
    })
}

/// Plain English rendering. Phase 6 adds the Urdu, Punjabi, Pashto and Sindhi versions;
/// this one is the fallback and is handy in logs and tests.
pub fn render_english(digest: &Digest) -> String {
    let s = &digest.summary;
    let t = &digest.tasks;

    let header = if s.from == s.to {
        format!("Report for {}", s.from.format("%d %b %Y"))
    } else {
        format!(
            "Report from {} to {}",
            s.from.format("%d %b"),
            s.to.format("%d %b %Y")
        )
    };

    let mut lines = vec![
        header,
        format!(
            "Sales: {} (on credit {})",
            format_rupees(s.sales_paisa),
            format_rupees(s.credit_sales_paisa)
        ),
        format!(
            "Expenses: {} (on credit {})",
            format_rupees(s.expenses_paisa),
            format_rupees(s.credit_expenses_paisa)
        ),
        format!("Received from customers: {}", format_rupees(s.received_paisa)),
        format!("Paid to suppliers: {}", format_rupees(s.paid_paisa)),
        format!("Profit: {}", format_rupees(s.profit_paisa())),
        format!("Cash in hand: {}", format_rupees(s.cash_balance_paisa)),
        format!("Customers owe you: {}", format_rupees(s.receivable_paisa)),
        format!("You owe suppliers: {}", format_rupees(s.payable_paisa)),
        format!(
            "Tasks: {} open, {} overdue, {} finished",
            t.open, t.overdue, t.done_in_range
        ),
    ];

    if !digest.top_receivables.is_empty() {
        lines.push(format!(
            "Most to collect: {}",
            party_list(&digest.top_receivables)
        ));
    }
    if !digest.top_payables.is_empty() {
        lines.push(format!("Most to pay: {}", party_list(&digest.top_payables)));
    }

    lines.join("\n")
}

fn party_list(parties: &[Party]) -> String {
    parties
        .iter()
        .map(|p| format!("{} {}", p.name, format_rupees(p.balance_paisa.abs())))
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::task::Task,
        nlp::{pakistan_today, TxKind},
        services::{
            ledger::{record, NewEntry},
            test_support,
        },
    };
    use chrono::{Duration, Utc};

    fn party(name: &str, balance_paisa: i64) -> Party {
        Party {
            id: Uuid::new_v4(),
            business_id: Uuid::new_v4(),
            name: name.to_string(),
            balance_paisa,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn renders_a_readable_english_report() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        let digest = Digest {
            summary: Summary {
                from: day,
                to: day,
                entries: 5,
                sales_paisa: 350_000,
                credit_sales_paisa: 250_000,
                expenses_paisa: 30_000,
                credit_expenses_paisa: 0,
                received_paisa: 100_000,
                paid_paisa: 0,
                cash_balance_paisa: 170_000,
                receivable_paisa: 150_000,
                payable_paisa: 500_000,
            },
            tasks: TaskStats {
                open: 2,
                overdue: 1,
                done_in_range: 3,
                created_in_range: 4,
            },
            top_receivables: vec![party("Ali", 150_000)],
            top_payables: vec![party("Supplier Co", -500_000)],
        };

        let text = render_english(&digest);
        assert!(text.starts_with("Report for 22 Nov 2026"));
        assert!(text.contains("Sales: Rs 3,500 (on credit Rs 2,500)"));
        assert!(text.contains("Profit: Rs 3,200"));
        assert!(text.contains("Cash in hand: Rs 1,700"));
        assert!(text.contains("Customers owe you: Rs 1,500"));
        assert!(text.contains("Tasks: 2 open, 1 overdue, 3 finished"));
        assert!(text.contains("Most to collect: Ali Rs 1,500"));
        assert!(text.contains("Most to pay: Supplier Co Rs 5,000"));
    }

    #[tokio::test]
    async fn digest_combines_ledger_and_tasks() {
        let Some(pool) = test_support::pool().await else {
            return;
        };
        let (business, owner) = test_support::business_with_owner(&pool).await;
        let bilal = test_support::add_employee(&pool, business.id, "Bilal").await;
        let today = pakistan_today();
        let yesterday = today - Duration::days(1);

        // Ledger: Rs 1,000 credit sale to Ali.
        record(
            &pool,
            &owner,
            NewEntry {
                kind: TxKind::Sale,
                amount_paisa: 100_000,
                on_credit: true,
                party_name: Some("Ali".to_string()),
                description: None,
                transcript: None,
                occurred_on: today,
            },
        )
        .await
        .unwrap();

        // Tasks: one overdue, one open without a date, one finished.
        Task::insert(&pool, business.id, owner.id, bilal.id, "Overdue job", Some(yesterday))
            .await
            .unwrap();
        Task::insert(&pool, business.id, owner.id, bilal.id, "Open job", None)
            .await
            .unwrap();
        let finished = Task::insert(&pool, business.id, owner.id, bilal.id, "Finished job", None)
            .await
            .unwrap();
        Task::mark_done(&pool, finished.id).await.unwrap();

        let digest = build_digest(&pool, business.id, Period::Today, today)
            .await
            .unwrap();

        assert_eq!(digest.summary.sales_paisa, 100_000);
        assert_eq!(digest.summary.receivable_paisa, 100_000);
        assert_eq!(digest.top_receivables.len(), 1);
        assert_eq!(digest.top_receivables[0].name, "Ali");
        assert!(digest.top_payables.is_empty());
        assert_eq!(
            digest.tasks,
            TaskStats {
                open: 2,
                overdue: 1,
                done_in_range: 1,
                created_in_range: 3,
            }
        );

        test_support::cleanup(&pool, business.id).await;
    }
}