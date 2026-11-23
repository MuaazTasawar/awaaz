use chrono::{Duration, NaiveDate};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{AppError, Result},
    models::{
        party::normalize_party_name,
        task::{Task, TaskWithAssignee},
        user::User,
    },
};

const MAX_TITLE_CHARS: usize = 200;
const MAX_DAYS_AHEAD: i64 = 365;
const MAX_CHOICES: usize = 5;

/// Result of matching a spoken employee name.
#[derive(Debug, Clone)]
pub enum AssigneeMatch {
    NotFound,
    One(User),
    /// Several employees fit ("Bilal" when both "Bilal Ahmed" and "Bilal Raza" work here).
    /// The bot asks which one.
    Many(Vec<User>),
}

/// Result of trying to finish a task.
#[derive(Debug, Clone)]
pub enum CompleteOutcome {
    Done(Task),
    /// There are no open tasks in scope at all.
    NothingOpen,
    /// There are open tasks, but none fits the description.
    NoMatch,
    /// Several fit; the bot asks which one.
    Ambiguous(Vec<TaskWithAssignee>),
}

fn to_match(mut users: Vec<User>) -> AssigneeMatch {
    match users.len() {
        0 => AssigneeMatch::NotFound,
        1 => AssigneeMatch::One(users.remove(0)),
        _ => AssigneeMatch::Many(users),
    }
}

/// Match a spoken name against the people of one business. Exact (after normalizing spelling)
/// wins; otherwise a name that contains, or is contained in, what was spoken
/// ("Bilal bhai" matches "Bilal").
pub fn match_users(users: &[User], spoken: &str) -> AssigneeMatch {
    let wanted = normalize_party_name(spoken);
    if wanted.chars().count() < 2 {
        return AssigneeMatch::NotFound;
    }

    let normalized: Vec<(String, &User)> = users
        .iter()
        .map(|user| (normalize_party_name(&user.name), user))
        .collect();

    let exact: Vec<User> = normalized
        .iter()
        .filter(|entry| entry.0 == wanted)
        .map(|entry| entry.1.clone())
        .collect();
    if !exact.is_empty() {
        return to_match(exact);
    }

    let partial: Vec<User> = normalized
        .iter()
        .filter(|entry| {
            entry.0.chars().count() >= 2
                && (entry.0.contains(wanted.as_str()) || wanted.contains(entry.0.as_str()))
        })
        .map(|entry| entry.1.clone())
        .collect();
    to_match(partial)
}

/// Find the employee a spoken name refers to.
pub async fn resolve_assignee(
    pool: &PgPool,
    business_id: Uuid,
    spoken: &str,
) -> Result<AssigneeMatch> {
    let users = User::list_for_business(pool, business_id).await?;
    Ok(match_users(&users, spoken))
}

/// Which open tasks does a spoken description point to? No description means all of them.
/// First the title must contain the description (or the other way round); failing that,
/// a shared word of three or more letters is enough.
pub fn pick_tasks(tasks: &[TaskWithAssignee], hint: Option<&str>) -> Vec<TaskWithAssignee> {
    let hint_norm = hint.map(normalize_party_name).unwrap_or_default();
    if hint_norm.is_empty() {
        return tasks.to_vec();
    }

    let contained: Vec<TaskWithAssignee> = tasks
        .iter()
        .filter(|t| {
            let title = normalize_party_name(&t.task.title);
            title.contains(hint_norm.as_str()) || hint_norm.contains(title.as_str())
        })
        .cloned()
        .collect();
    if !contained.is_empty() {
        return contained;
    }

    let hint_words: Vec<&str> = hint_norm
        .split_whitespace()
        .filter(|word| word.chars().count() >= 3)
        .collect();

    tasks
        .iter()
        .filter(|t| {
            let title = normalize_party_name(&t.task.title);
            title
                .split_whitespace()
                .any(|word| hint_words.contains(&word))
        })
        .cloned()
        .collect()
}

/// Owners see (and may filter) every task of the business; employees only their own.
fn scope(actor: &User, assignee: Option<&User>) -> Option<Uuid> {
    if actor.is_owner() {
        assignee.map(|a| a.id)
    } else {
        Some(actor.id)
    }
}

/// Give a task to an employee. Only owners may do this, only inside their own business,
/// and a due date cannot be in the past or more than a year ahead.
pub async fn assign(
    pool: &PgPool,
    owner: &User,
    assignee: &User,
    title: &str,
    due_date: Option<NaiveDate>,
    today: NaiveDate,
) -> Result<Task> {
    if !owner.is_owner() {
        return Err(AppError::Unauthorized);
    }
    if !assignee.is_active || assignee.business_id != owner.business_id {
        return Err(AppError::BadRequest("unknown employee".to_string()));
    }

    let title = title.trim();
    let len = title.chars().count();
    if !(2..=MAX_TITLE_CHARS).contains(&len) {
        return Err(AppError::BadRequest(
            "task description must be 2 to 200 characters".to_string(),
        ));
    }

    if let Some(due) = due_date {
        if due < today {
            return Err(AppError::BadRequest("due date is in the past".to_string()));
        }
        if due > today + Duration::days(MAX_DAYS_AHEAD) {
            return Err(AppError::BadRequest(
                "due date is too far ahead".to_string(),
            ));
        }
    }

    Ok(Task::insert(
        pool,
        owner.business_id,
        owner.id,
        assignee.id,
        title,
        due_date,
    )
    .await?)
}

/// Open tasks the actor may see, most urgent first.
pub async fn list_open(
    pool: &PgPool,
    actor: &User,
    assignee: Option<&User>,
) -> Result<Vec<TaskWithAssignee>> {
    Ok(Task::list_open(pool, actor.business_id, scope(actor, assignee)).await?)
}

/// Finish a task described by `hint` (words from the voice note) and/or an employee.
/// Employees can only finish their own tasks. If exactly one task fits it is finished;
/// if several fit, they are returned so the bot can ask which one was meant.
pub async fn complete(
    pool: &PgPool,
    actor: &User,
    hint: Option<&str>,
    assignee: Option<&User>,
) -> Result<CompleteOutcome> {
    let open = Task::list_open(pool, actor.business_id, scope(actor, assignee)).await?;
    if open.is_empty() {
        return Ok(CompleteOutcome::NothingOpen);
    }

    let mut candidates = pick_tasks(&open, hint);
    match candidates.len() {
        0 => Ok(CompleteOutcome::NoMatch),
        1 => {
            let chosen = candidates.remove(0);
            match Task::mark_done(pool, chosen.task.id).await? {
                Some(done) => Ok(CompleteOutcome::Done(done)),
                // Someone else finished it a moment ago.
                None => Ok(CompleteOutcome::NothingOpen),
            }
        }
        _ => {
            candidates.truncate(MAX_CHOICES);
            Ok(CompleteOutcome::Ambiguous(candidates))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::task::STATUS_OPEN,
        nlp::pakistan_today,
        services::test_support,
    };
    use chrono::Utc;

    // ---------- pure logic ----------

    fn user(name: &str) -> User {
        User {
            id: Uuid::new_v4(),
            business_id: Uuid::new_v4(),
            phone: "923000000000".to_string(),
            name: name.to_string(),
            role: "employee".to_string(),
            language: "ur".to_string(),
            is_active: true,
            created_at: Utc::now(),
        }
    }

    fn open_task(title: &str) -> TaskWithAssignee {
        TaskWithAssignee {
            task: Task {
                id: Uuid::new_v4(),
                business_id: Uuid::new_v4(),
                assigned_by: Uuid::new_v4(),
                assignee_id: Uuid::new_v4(),
                title: title.to_string(),
                status: STATUS_OPEN.to_string(),
                due_date: None,
                created_at: Utc::now(),
                completed_at: None,
            },
            assignee_name: "Bilal".to_string(),
        }
    }

    #[test]
    fn matches_spoken_employee_names() {
        let users = vec![user("Bilal Ahmed"), user("Usman Ali"), user("Bilal Raza")];

        assert!(matches!(match_users(&users, "usman"), AssigneeMatch::One(ref u) if u.name == "Usman Ali"));
        assert!(matches!(match_users(&users, "Bilal"), AssigneeMatch::Many(ref v) if v.len() == 2));
        assert!(matches!(match_users(&users, "bilal raza"), AssigneeMatch::One(ref u) if u.name == "Bilal Raza"));
        assert!(matches!(match_users(&users, "Bilal Ahmed bhai"), AssigneeMatch::One(ref u) if u.name == "Bilal Ahmed"));
        assert!(matches!(match_users(&users, "Zubair"), AssigneeMatch::NotFound));
        assert!(matches!(match_users(&users, "b"), AssigneeMatch::NotFound));
    }

    #[test]
    fn matches_urdu_spelling_variants() {
        // Stored with the Urdu yeh, spoken (and transcribed) with the Arabic yeh.
        let users = vec![user("\u{0639}\u{0644}\u{06CC}")];
        assert!(matches!(
            match_users(&users, "\u{0639}\u{0644}\u{064A}"),
            AssigneeMatch::One(_)
        ));
    }

    #[test]
    fn picks_tasks_by_spoken_description() {
        let tasks = vec![
            open_task("Deliver cloth to Lahore"),
            open_task("Count the stock"),
            open_task("Clean the shop"),
        ];

        assert_eq!(pick_tasks(&tasks, None).len(), 3);
        assert_eq!(pick_tasks(&tasks, Some("   ")).len(), 3);

        let by_contains = pick_tasks(&tasks, Some("lahore"));
        assert_eq!(by_contains.len(), 1);
        assert_eq!(by_contains[0].task.title, "Deliver cloth to Lahore");

        // No substring match, but the words "clean" and "shop" appear in a title.
        let by_words = pick_tasks(&tasks, Some("clean shop"));
        assert_eq!(by_words.len(), 1);
        assert_eq!(by_words[0].task.title, "Clean the shop");

        assert!(pick_tasks(&tasks, Some("xyz")).is_empty());
    }

    // ---------- database tests ----------

    #[tokio::test]
    async fn assign_list_and_complete() {
        let Some(pool) = test_support::pool().await else {
            return;
        };
        let (business, owner) = test_support::business_with_owner(&pool).await;
        let bilal = test_support::add_employee(&pool, business.id, "Bilal").await;
        let usman = test_support::add_employee(&pool, business.id, "Usman").await;
        let today = pakistan_today();
        let tomorrow = today + Duration::days(1);

        let lahore = assign(&pool, &owner, &bilal, "Deliver cloth to Lahore", Some(tomorrow), today)
            .await
            .unwrap();
        let stock = assign(&pool, &owner, &bilal, "Count the stock", None, today)
            .await
            .unwrap();

        // The owner sees both, tasks with a due date first.
        let all = list_open(&pool, &owner, None).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].task.id, lahore.id);
        assert_eq!(all[0].assignee_name, "Bilal");

        // Bilal sees his two, Usman sees none, and the owner can filter by employee.
        assert_eq!(list_open(&pool, &bilal, None).await.unwrap().len(), 2);
        assert_eq!(list_open(&pool, &usman, None).await.unwrap().len(), 0);
        assert_eq!(list_open(&pool, &owner, Some(&usman)).await.unwrap().len(), 0);
        assert_eq!(list_open(&pool, &owner, Some(&bilal)).await.unwrap().len(), 2);

        // Usman cannot finish Bilal's task: he has no open tasks of his own.
        assert!(matches!(
            complete(&pool, &usman, Some("lahore"), None).await.unwrap(),
            CompleteOutcome::NothingOpen
        ));

        // Bilal finishes the Lahore delivery by describing it.
        match complete(&pool, &bilal, Some("lahore"), None).await.unwrap() {
            CompleteOutcome::Done(task) => {
                assert_eq!(task.id, lahore.id);
                assert_eq!(task.status, "done");
                assert!(task.completed_at.is_some());
            }
            other => panic!("expected Done, got {other:?}"),
        }

        // A description that fits nothing.
        assert!(matches!(
            complete(&pool, &bilal, Some("xyz"), None).await.unwrap(),
            CompleteOutcome::NoMatch
        ));

        // The owner finishes the last open task without describing it.
        match complete(&pool, &owner, None, None).await.unwrap() {
            CompleteOutcome::Done(task) => assert_eq!(task.id, stock.id),
            other => panic!("expected Done, got {other:?}"),
        }
        assert!(matches!(
            complete(&pool, &owner, None, None).await.unwrap(),
            CompleteOutcome::NothingOpen
        ));

        test_support::cleanup(&pool, business.id).await;
    }

    #[tokio::test]
    async fn ambiguity_and_permissions() {
        let Some(pool) = test_support::pool().await else {
            return;
        };
        let (business, owner) = test_support::business_with_owner(&pool).await;
        let bilal = test_support::add_employee(&pool, business.id, "Bilal").await;
        let usman = test_support::add_employee(&pool, business.id, "Usman").await;
        let (other_business, other_owner) = test_support::business_with_owner(&pool).await;
        let today = pakistan_today();

        assign(&pool, &owner, &bilal, "Clean the shop", None, today)
            .await
            .unwrap();
        assign(&pool, &owner, &bilal, "Clean the store room", None, today)
            .await
            .unwrap();

        // "clean" fits both tasks, so the bot must ask which one.
        match complete(&pool, &owner, Some("clean"), None).await.unwrap() {
            CompleteOutcome::Ambiguous(list) => assert_eq!(list.len(), 2),
            other => panic!("expected Ambiguous, got {other:?}"),
        }

        // An employee cannot assign work.
        let by_employee = assign(&pool, &bilal, &usman, "Do something", None, today).await;
        assert!(matches!(by_employee, Err(AppError::Unauthorized)));

        // A due date in the past is refused.
        let yesterday = today - Duration::days(1);
        let past = assign(&pool, &owner, &bilal, "Late task", Some(yesterday), today).await;
        assert!(matches!(past, Err(AppError::BadRequest(_))));

        // An employee of another business cannot be given work.
        let foreign = assign(&pool, &owner, &other_owner, "Wrong shop", None, today).await;
        assert!(matches!(foreign, Err(AppError::BadRequest(_))));

        // Too-short titles are refused.
        let short = assign(&pool, &owner, &bilal, " a ", None, today).await;
        assert!(matches!(short, Err(AppError::BadRequest(_))));

        test_support::cleanup(&pool, business.id).await;
        test_support::cleanup(&pool, other_business.id).await;
    }
}