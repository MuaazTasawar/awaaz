use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

pub const STATUS_OPEN: &str = "open";
pub const STATUS_DONE: &str = "done";
pub const STATUS_CANCELLED: &str = "cancelled";

const TASK_COLUMNS: &str = "id, business_id, assigned_by, assignee_id, title, status, due_date, \
                            created_at, completed_at";

/// A piece of work an owner gave to an employee.
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Task {
    pub id: Uuid,
    pub business_id: Uuid,
    pub assigned_by: Uuid,
    pub assignee_id: Uuid,
    pub title: String,
    pub status: String,
    pub due_date: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// A task together with the employee's name, for lists shown to the user.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaskWithAssignee {
    #[sqlx(flatten)]
    pub task: Task,
    pub assignee_name: String,
}

impl Task {
    pub fn is_open(&self) -> bool {
        self.status == STATUS_OPEN
    }

    /// Open and its due date is before `today` (Pakistan date).
    pub fn is_overdue(&self, today: NaiveDate) -> bool {
        self.is_open() && self.due_date.map(|due| due < today).unwrap_or(false)
    }

    pub async fn insert(
        pool: &PgPool,
        business_id: Uuid,
        assigned_by: Uuid,
        assignee_id: Uuid,
        title: &str,
        due_date: Option<NaiveDate>,
    ) -> Result<Task, sqlx::Error> {
        let sql = format!(
            "INSERT INTO tasks (business_id, assigned_by, assignee_id, title, due_date)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING {TASK_COLUMNS}"
        );
        sqlx::query_as::<_, Task>(&sql)
            .bind(business_id)
            .bind(assigned_by)
            .bind(assignee_id)
            .bind(title)
            .bind(due_date)
            .fetch_one(pool)
            .await
    }

    pub async fn find(pool: &PgPool, id: Uuid) -> Result<Option<Task>, sqlx::Error> {
        let sql = format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = $1");
        sqlx::query_as::<_, Task>(&sql)
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    /// Mark an open task finished. Returns None if it was already finished or cancelled,
    /// which also protects against two people finishing the same task at once.
    pub async fn mark_done(pool: &PgPool, id: Uuid) -> Result<Option<Task>, sqlx::Error> {
        let sql = format!(
            "UPDATE tasks SET status = 'done', completed_at = now()
             WHERE id = $1 AND status = 'open'
             RETURNING {TASK_COLUMNS}"
        );
        sqlx::query_as::<_, Task>(&sql)
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    /// Open tasks of a business, optionally only those of one employee.
    /// Tasks with a due date come first, soonest first, then the rest oldest first.
    pub async fn list_open(
        pool: &PgPool,
        business_id: Uuid,
        assignee_id: Option<Uuid>,
    ) -> Result<Vec<TaskWithAssignee>, sqlx::Error> {
        sqlx::query_as::<_, TaskWithAssignee>(
            "SELECT t.id, t.business_id, t.assigned_by, t.assignee_id, t.title, t.status,
                    t.due_date, t.created_at, t.completed_at,
                    u.name AS assignee_name
             FROM tasks t
             JOIN users u ON u.id = t.assignee_id
             WHERE t.business_id = $1
               AND t.status = 'open'
               AND ($2::uuid IS NULL OR t.assignee_id = $2)
             ORDER BY (t.due_date IS NULL), t.due_date, t.created_at
             LIMIT 50",
        )
        .bind(business_id)
        .bind(assignee_id)
        .fetch_all(pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(due: Option<NaiveDate>, status: &str) -> Task {
        Task {
            id: Uuid::new_v4(),
            business_id: Uuid::new_v4(),
            assigned_by: Uuid::new_v4(),
            assignee_id: Uuid::new_v4(),
            title: "x".to_string(),
            status: status.to_string(),
            due_date: due,
            created_at: Utc::now(),
            completed_at: None,
        }
    }

    #[test]
    fn overdue_only_counts_open_tasks_past_their_due_date() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        let yesterday = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();

        assert!(task(Some(yesterday), STATUS_OPEN).is_overdue(today));
        assert!(!task(Some(today), STATUS_OPEN).is_overdue(today));
        assert!(!task(None, STATUS_OPEN).is_overdue(today));
        assert!(!task(Some(yesterday), STATUS_DONE).is_overdue(today));
        assert!(!task(Some(yesterday), STATUS_CANCELLED).is_overdue(today));
    }
}