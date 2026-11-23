//! What happens after the user says Yes (or No) to a read-back.

use crate::{
    error::{AppError, Result},
    i18n::{self, Lang},
    models::{
        pending_action::{
            Action, PendingAction, STATUS_CANCELLED, STATUS_CONFIRMED,
        },
        task::Task,
        user::User,
    },
    nlp::pakistan_today,
    services::{
        ledger::{self, NewEntry},
        onboarding::{self, AddEmployeeOutcome},
        tasks,
    },
    state::AppState,
};

async fn send(state: &AppState, user: &User, text: &str) -> Result<()> {
    state.whatsapp.send_text(&user.phone, text).await
}

/// Carry out a confirmed action. The pending item is claimed first, so tapping Yes twice,
/// or saying it twice, can never save an entry twice.
pub async fn confirm(
    state: &AppState,
    user: &User,
    pending: &PendingAction,
    action: Action,
) -> Result<()> {
    let lang = Lang::from_code(&user.language);

    if !PendingAction::claim(&state.db, pending.id, STATUS_CONFIRMED).await? {
        return send(state, user, &i18n::nothing_to_confirm(lang)).await;
    }

    match execute(state, user, pending, action).await {
        Ok(text) => send(state, user, &text).await,
        Err(AppError::Unauthorized) => send(state, user, &i18n::not_allowed(lang)).await,
        Err(AppError::BadRequest(reason)) => {
            tracing::info!(%reason, "confirmed action was refused");
            send(state, user, &i18n::error(lang)).await
        }
        Err(e) => {
            // A technical failure: reopen the item so the user can say Yes again.
            PendingAction::release(&state.db, pending.id).await.ok();
            Err(e)
        }
    }
}

/// The user said No (or cancel).
pub async fn cancel(state: &AppState, user: &User, pending: &PendingAction) -> Result<()> {
    let lang = Lang::from_code(&user.language);
    PendingAction::claim(&state.db, pending.id, STATUS_CANCELLED).await?;
    send(state, user, &i18n::cancelled(lang)).await
}

async fn execute(
    state: &AppState,
    user: &User,
    pending: &PendingAction,
    action: Action,
) -> Result<String> {
    let lang = Lang::from_code(&user.language);
    let today = pakistan_today();

    match action {
        Action::Record {
            kind,
            amount_paisa,
            on_credit,
            party_name,
            description,
            ..
        } => {
            let recorded = ledger::record(
                &state.db,
                user,
                NewEntry {
                    kind,
                    amount_paisa,
                    on_credit,
                    party_name,
                    description,
                    transcript: Some(pending.transcript.clone()),
                    occurred_on: today,
                },
            )
            .await?;

            Ok(i18n::recorded(
                lang,
                kind,
                amount_paisa,
                recorded.transaction.on_credit,
                recorded
                    .party
                    .as_ref()
                    .map(|p| (p.name.as_str(), p.balance_paisa)),
                recorded.cash_balance_paisa,
            ))
        }

        Action::Assign {
            assignee_id,
            assignee_name,
            title,
            due_date,
        } => {
            let assignee = User::find(&state.db, assignee_id)
                .await?
                .ok_or_else(|| AppError::NotFound("employee".to_string()))?;
            let task = tasks::assign(&state.db, user, &assignee, &title, due_date, today).await?;

            // Tell the employee. WhatsApp only allows this within 24 hours of their last message
            // to us, so it may fail; the owner is told when that happens.
            let notified = if assignee.id == user.id {
                true
            } else {
                let text = i18n::task_notice(
                    Lang::from_code(&assignee.language),
                    &user.name,
                    &task.title,
                    task.due_date,
                );
                match state.whatsapp.send_text(&assignee.phone, &text).await {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::warn!(error = %e, "could not notify the employee");
                        false
                    }
                }
            };

            Ok(i18n::task_assigned(lang, &assignee_name, notified))
        }

        Action::Complete { task_id, .. } => {
            let task = Task::find(&state.db, task_id)
                .await?
                .ok_or_else(|| AppError::NotFound("task".to_string()))?;

            let allowed = task.business_id == user.business_id
                && (user.is_owner() || task.assignee_id == user.id);
            if !allowed {
                return Err(AppError::Unauthorized);
            }

            match Task::mark_done(&state.db, task_id).await? {
                None => Ok(i18n::task_no_match(lang)),
                Some(done) => {
                    // Let the person who assigned it know.
                    if done.assigned_by != user.id {
                        if let Some(owner) = User::find(&state.db, done.assigned_by).await? {
                            let text = i18n::task_done_notice(
                                Lang::from_code(&owner.language),
                                &user.name,
                                &done.title,
                            );
                            if let Err(e) = state.whatsapp.send_text(&owner.phone, &text).await {
                                tracing::warn!(error = %e, "could not notify the task owner");
                            }
                        }
                    }
                    Ok(i18n::task_done(lang, &done.title))
                }
            }
        }

        Action::AddEmployee { name, phone } => {
            match onboarding::add_employee(&state.db, user, &name, &phone).await? {
                AddEmployeeOutcome::Created(employee) => {
                    let welcome = i18n::welcome(
                        Lang::from_code(&employee.language),
                        &employee.name,
                    );
                    if let Err(e) = state.whatsapp.send_text(&employee.phone, &welcome).await {
                        tracing::warn!(error = %e, "could not send the welcome message");
                    }
                    Ok(i18n::employee_added(lang, &employee.name))
                }
                AddEmployeeOutcome::PhoneTaken => Ok(i18n::employee_taken(lang)),
                AddEmployeeOutcome::Invalid => Ok(i18n::didnt_understand(lang)),
            }
        }
    }
}