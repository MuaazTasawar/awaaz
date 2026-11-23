//! The heart of Awaaz: one incoming WhatsApp message goes in, a reply comes out.
//!
//!   who is it?  ->  voice to text  ->  is it an answer to my last question?
//!   ->  understand the command  ->  check permissions  ->  ask for missing details
//!   ->  match names to real people  ->  read back and wait for Yes  ->  (confirm.rs saves it)

use crate::{
    error::{AppError, Result},
    handlers::confirm,
    i18n::{self, Lang},
    models::{
        business::is_supported_language,
        party::PartyMatch,
        pending_action::{Action, PendingAction, PendingState, STATUS_CANCELLED},
        user::User,
    },
    nlp::{pakistan_today, Command, ParseContext, ParsedCommand},
    services::{
        ledger, onboarding, reports,
        tasks::{self, AssigneeMatch},
    },
    state::AppState,
    whatsapp::{IncomingMessage, MessageContent, BTN_CONFIRM_NO, BTN_CONFIRM_YES},
};

/// After this many follow-up questions in a row the bot gives up and asks to start again.
const MAX_ASK_ATTEMPTS: u8 = 3;
/// The saved transcript is capped so repeated follow-ups cannot grow without limit.
const MAX_TRANSCRIPT_KEEP: usize = 1500;

/// Entry point called by the webhook for every new message.
pub async fn handle_message(state: AppState, msg: IncomingMessage) -> Result<()> {
    let user = match User::find_active_by_phone(&state.db, &msg.from).await? {
        Some(user) => user,
        None => return onboard_or_reject(&state, &msg).await,
    };

    state.whatsapp.mark_read(&msg.id).await.ok();

    if let Err(e) = process(&state, &user, &msg).await {
        let lang = Lang::from_code(&user.language);
        state
            .whatsapp
            .send_text(&user.phone, &i18n::error(lang))
            .await
            .ok();
        return Err(e);
    }
    Ok(())
}

/// A number we have never seen. Allowlisted numbers become owners; everyone else is told
/// (in Urdu and English) to ask their employer.
async fn onboard_or_reject(state: &AppState, msg: &IncomingMessage) -> Result<()> {
    if state.config.is_allowed_owner(&msg.from) {
        match onboarding::register_owner(&state.db, &msg.from, msg.sender_name.as_deref()).await {
            Ok(user) => {
                tracing::info!(user = %user.id, "new owner registered");
                let text = i18n::welcome(Lang::Ur, &user.name);
                return state.whatsapp.send_text(&user.phone, &text).await;
            }
            Err(e) => {
                // Two first messages at once: the other one already registered this number.
                if User::find_active_by_phone(&state.db, &msg.from).await?.is_some() {
                    return Ok(());
                }
                return Err(e);
            }
        }
    }

    tracing::info!("message from an unregistered number");
    let text = format!(
        "{}\n\n{}",
        i18n::not_registered(Lang::Ur),
        i18n::not_registered(Lang::En)
    );
    state.whatsapp.send_text(&msg.from, &text).await
}

async fn reply(state: &AppState, user: &User, text: &str) -> Result<()> {
    state.whatsapp.send_text(&user.phone, text).await
}

async fn process(state: &AppState, user: &User, msg: &IncomingMessage) -> Result<()> {
    let lang = Lang::from_code(&user.language);

    match &msg.content {
        MessageContent::ButtonReply { id, .. } => handle_button(state, user, id).await,

        MessageContent::Text(text) => handle_text(state, user, text, None).await,

        MessageContent::Audio {
            media_id,
            mime_type,
        } => {
            let (bytes, downloaded_mime) = state.whatsapp.download_media(media_id).await?;
            let mime = if downloaded_mime.is_empty() {
                mime_type.as_str()
            } else {
                downloaded_mime.as_str()
            };

            // No language hint: Whisper detects the language itself, so nobody has to
            // configure anything and a speaker can switch languages freely.
            let transcript = match state.stt.transcribe(bytes, mime, None).await {
                Ok(transcript) => transcript,
                Err(AppError::Stt(reason)) => {
                    tracing::info!(%reason, "voice note had no usable speech");
                    return reply(state, user, &i18n::didnt_understand(lang)).await;
                }
                Err(e) => return Err(e),
            };

            tracing::debug!(
                transcript = %transcript.text,
                language = ?transcript.language,
                "voice note transcribed"
            );
            handle_text(state, user, &transcript.text, transcript.language.as_deref()).await
        }

        MessageContent::Unsupported(kind) => {
            tracing::debug!(%kind, "unsupported message type");
            reply(state, user, &i18n::didnt_understand(lang)).await
        }
    }
}

/// The user tapped Yes or No under a read-back.
async fn handle_button(state: &AppState, user: &User, button_id: &str) -> Result<()> {
    let lang = Lang::from_code(&user.language);

    let Some(pending) = PendingAction::find_active(&state.db, user.id).await? else {
        return reply(state, user, &i18n::nothing_to_confirm(lang)).await;
    };
    let Some(PendingState::Confirm { action }) = pending.state() else {
        return reply(state, user, &i18n::nothing_to_confirm(lang)).await;
    };

    match button_id {
        BTN_CONFIRM_YES => confirm::confirm(state, user, &pending, action).await,
        BTN_CONFIRM_NO => confirm::cancel(state, user, &pending).await,
        _ => reply(state, user, &i18n::nothing_to_confirm(lang)).await,
    }
}

/// Text from a voice transcript or typed by the user.
async fn handle_text(
    state: &AppState,
    user: &User,
    text: &str,
    detected_language: Option<&str>,
) -> Result<()> {
    let lang = Lang::from_code(&user.language);
    let text = text.trim();
    if text.is_empty() {
        return reply(state, user, &i18n::didnt_understand(lang)).await;
    }

    let mut transcript = text.to_string();
    let mut attempts: u8 = 0;

    // Is this the answer to a question the bot asked earlier?
    if let Some(pending) = PendingAction::find_active(&state.db, user.id).await? {
        match pending.state() {
            Some(PendingState::Confirm { action }) => match i18n::parse_yes_no(text) {
                Some(true) => return confirm::confirm(state, user, &pending, action).await,
                Some(false) => return confirm::cancel(state, user, &pending).await,
                // Something else entirely: drop the old read-back and treat this as a new command.
                None => {
                    PendingAction::claim(&state.db, pending.id, STATUS_CANCELLED).await?;
                }
            },
            Some(PendingState::NeedInfo { attempts: so_far }) => {
                if i18n::parse_yes_no(text) == Some(false) {
                    return confirm::cancel(state, user, &pending).await;
                }
                // The new words are added to the old ones and understood together:
                // "Ali ko maal udhaar becha" + "dhai hazaar" = a complete entry.
                transcript = keep_tail(&format!("{} {}", pending.transcript, text));
                attempts = so_far;
            }
            None => {
                PendingAction::claim(&state.db, pending.id, STATUS_CANCELLED).await?;
            }
        }
    }

    interpret(state, user, &transcript, attempts, detected_language).await
}

fn keep_tail(text: &str) -> String {
    let count = text.chars().count();
    if count <= MAX_TRANSCRIPT_KEEP {
        text.to_string()
    } else {
        text.chars().skip(count - MAX_TRANSCRIPT_KEEP).collect()
    }
}

/// Understand the transcript and carry out whatever it asks.
async fn interpret(
    state: &AppState,
    user: &User,
    transcript: &str,
    attempts: u8,
    detected_language: Option<&str>,
) -> Result<()> {
    let employee_names: Vec<String> = User::list_for_business(&state.db, user.business_id)
        .await?
        .into_iter()
        .map(|u| u.name)
        .collect();
    let party_names = ledger::known_party_names(&state.db, user.business_id).await?;

    let ctx = ParseContext {
        today: pakistan_today(),
        employee_names,
        party_names,
    };

    let parsed = state.parser.parse(transcript, &ctx).await?;
    let lang = settle_language(state, user, &parsed, detected_language).await?;
    dispatch(state, user, lang, parsed.command, transcript, attempts).await
}

/// Replies follow the language the person actually speaks, but only when the speech engine
/// and the language model agree on it, so one odd transcript cannot flip a user's language.
async fn settle_language(
    state: &AppState,
    user: &User,
    parsed: &ParsedCommand,
    detected: Option<&str>,
) -> Result<Lang> {
    if let (Some(spoken), Some(heard)) = (parsed.language.as_deref(), detected) {
        if spoken == heard && spoken != user.language && is_supported_language(spoken) {
            User::set_language(&state.db, user.id, spoken).await?;
            return Ok(Lang::from_code(spoken));
        }
    }
    Ok(Lang::from_code(&user.language))
}

/// Commands only the owner may give. Employees can finish and list their own tasks,
/// ask for help and change their language.
fn owner_only(command: &Command) -> bool {
    matches!(
        command,
        Command::RecordTransaction { .. }
            | Command::AssignTask { .. }
            | Command::AddEmployee { .. }
            | Command::QuerySummary { .. }
            | Command::QueryBalance
            | Command::QueryParty { .. }
    )
}

async fn dispatch(
    state: &AppState,
    user: &User,
    lang: Lang,
    command: Command,
    transcript: &str,
    attempts: u8,
) -> Result<()> {
    if !user.is_owner() && owner_only(&command) {
        return reply(state, user, &i18n::not_allowed(lang)).await;
    }

    if let Some(missing) = command.missing_info() {
        let question = i18n::ask(lang, missing);
        return ask_again(state, user, lang, &question, transcript, attempts).await;
    }

    let amount_paisa = command.amount_paisa();
    let due_date = command.due_date();
    let today = pakistan_today();

    match command {
        Command::Unclear { reason } => {
            tracing::debug!(?reason, "command not understood");
            reply(state, user, &i18n::didnt_understand(lang)).await
        }

        Command::Help => reply(state, user, &i18n::help(lang)).await,

        Command::SetLanguage { language } => {
            let code = language.unwrap_or_default();
            User::set_language(&state.db, user.id, &code).await?;
            reply(state, user, &i18n::language_changed(Lang::from_code(&code))).await
        }

        Command::QuerySummary { period } => {
            let period = period.unwrap_or_default();
            let digest = reports::build_digest(&state.db, user.business_id, period, today).await?;
            reply(state, user, &i18n::digest(lang, &digest, period)).await
        }

        Command::QueryBalance => {
            let cash = ledger::cash_balance(&state.db, user.business_id).await?;
            let (receivable, payable) = ledger::party_totals(&state.db, user.business_id).await?;
            reply(state, user, &i18n::balance(lang, cash, receivable, payable)).await
        }

        Command::QueryParty { party_name } => {
            let name = party_name.unwrap_or_default();
            match ledger::find_party(&state.db, user.business_id, &name).await? {
                PartyMatch::One(party) => {
                    let statement = ledger::party_statement(&state.db, party, 5).await?;
                    reply(state, user, &i18n::party_statement(lang, &statement)).await
                }
                PartyMatch::Many(list) => {
                    let names: Vec<String> = list.into_iter().map(|p| p.name).collect();
                    let question = i18n::choose_one(lang, &names);
                    ask_again(state, user, lang, &question, transcript, attempts).await
                }
                PartyMatch::NotFound => reply(state, user, &i18n::party_unknown(lang, &name)).await,
            }
        }

        Command::QueryTasks { assignee_name } => {
            let filter = match resolve_filter(
                state,
                user,
                lang,
                assignee_name.as_deref(),
                transcript,
                attempts,
            )
            .await?
            {
                Filter::Ready(filter) => filter,
                Filter::Replied => return Ok(()),
            };
            let list = tasks::list_open(&state.db, user, filter.as_ref()).await?;
            reply(state, user, &i18n::task_list(lang, &list, today)).await
        }

        Command::RecordTransaction {
            kind,
            on_credit,
            party_name,
            description,
            ..
        } => {
            let Some(amount_paisa) = amount_paisa else {
                return reply(state, user, &i18n::didnt_understand(lang)).await;
            };

            let mut canonical: Option<String> = None;
            let mut new_party = false;
            if let Some(spoken) = party_name.as_deref() {
                match ledger::find_party(&state.db, user.business_id, spoken).await? {
                    PartyMatch::One(party) => canonical = Some(party.name),
                    PartyMatch::Many(list) => {
                        let names: Vec<String> = list.into_iter().map(|p| p.name).collect();
                        let question = i18n::choose_one(lang, &names);
                        return ask_again(state, user, lang, &question, transcript, attempts)
                            .await;
                    }
                    PartyMatch::NotFound => {
                        canonical = Some(spoken.to_string());
                        new_party = true;
                    }
                }
            }

            let action = Action::Record {
                kind,
                amount_paisa,
                on_credit: ledger::effective_on_credit(kind, on_credit),
                party_name: canonical,
                new_party,
                description,
            };
            ask_confirmation(state, user, lang, action, transcript).await
        }

        Command::AssignTask {
            assignee_name,
            title,
            ..
        } => {
            let (Some(spoken), Some(title)) = (assignee_name, title) else {
                return reply(state, user, &i18n::didnt_understand(lang)).await;
            };

            match tasks::resolve_assignee(&state.db, user.business_id, &spoken).await? {
                AssigneeMatch::NotFound => {
                    reply(state, user, &i18n::employee_not_found(lang, &spoken)).await
                }
                AssigneeMatch::Many(list) => {
                    let names: Vec<String> = list.into_iter().map(|u| u.name).collect();
                    let question = i18n::choose_one(lang, &names);
                    ask_again(state, user, lang, &question, transcript, attempts).await
                }
                AssigneeMatch::One(assignee) => {
                    // A due date in the past is almost certainly a misheard "tomorrow"; drop it.
                    let due_date = due_date.filter(|d| *d >= today);
                    let action = Action::Assign {
                        assignee_id: assignee.id,
                        assignee_name: assignee.name,
                        title,
                        due_date,
                    };
                    ask_confirmation(state, user, lang, action, transcript).await
                }
            }
        }

        Command::CompleteTask {
            task_hint,
            assignee_name,
        } => {
            let filter = match resolve_filter(
                state,
                user,
                lang,
                assignee_name.as_deref(),
                transcript,
                attempts,
            )
            .await?
            {
                Filter::Ready(filter) => filter,
                Filter::Replied => return Ok(()),
            };

            let open = tasks::list_open(&state.db, user, filter.as_ref()).await?;
            if open.is_empty() {
                return reply(state, user, &i18n::no_open_tasks(lang)).await;
            }

            let mut candidates = tasks::pick_tasks(&open, task_hint.as_deref());
            match candidates.len() {
                0 => reply(state, user, &i18n::task_no_match(lang)).await,
                1 => {
                    let chosen = candidates.remove(0);
                    let action = Action::Complete {
                        task_id: chosen.task.id,
                        title: chosen.task.title.clone(),
                        assignee_name: chosen.assignee_name.clone(),
                    };
                    ask_confirmation(state, user, lang, action, transcript).await
                }
                _ => {
                    let names: Vec<String> = candidates
                        .iter()
                        .take(5)
                        .map(|c| format!("{} ({})", c.task.title, c.assignee_name))
                        .collect();
                    let question = i18n::choose_one(lang, &names);
                    ask_again(state, user, lang, &question, transcript, attempts).await
                }
            }
        }

        Command::AddEmployee { name, phone } => {
            let (Some(name), Some(phone)) = (name, phone) else {
                return reply(state, user, &i18n::didnt_understand(lang)).await;
            };

            if onboarding::phone_in_use(&state.db, &phone).await? {
                return reply(state, user, &i18n::employee_taken(lang)).await;
            }
            let action = Action::AddEmployee { name, phone };
            ask_confirmation(state, user, lang, action, transcript).await
        }
    }
}

/// Result of working out which employee a spoken name means for a list or a completion.
enum Filter {
    /// No filter (None) or a specific employee (Some).
    Ready(Option<User>),
    /// A reply was already sent (not found, or a "which one?" question).
    Replied,
}

/// Owners may name an employee ("Bilal ke kaam"); for employees the name is ignored
/// because they only ever see their own tasks.
async fn resolve_filter(
    state: &AppState,
    user: &User,
    lang: Lang,
    assignee_name: Option<&str>,
    transcript: &str,
    attempts: u8,
) -> Result<Filter> {
    if !user.is_owner() {
        return Ok(Filter::Ready(None));
    }
    let Some(name) = assignee_name else {
        return Ok(Filter::Ready(None));
    };

    match tasks::resolve_assignee(&state.db, user.business_id, name).await? {
        AssigneeMatch::One(found) => Ok(Filter::Ready(Some(found))),
        AssigneeMatch::NotFound => {
            reply(state, user, &i18n::employee_not_found(lang, name)).await?;
            Ok(Filter::Replied)
        }
        AssigneeMatch::Many(list) => {
            let names: Vec<String> = list.into_iter().map(|u| u.name).collect();
            let question = i18n::choose_one(lang, &names);
            ask_again(state, user, lang, &question, transcript, attempts).await?;
            Ok(Filter::Replied)
        }
    }
}

/// Ask a follow-up question and remember what was said so far. After too many rounds, stop.
async fn ask_again(
    state: &AppState,
    user: &User,
    lang: Lang,
    question: &str,
    transcript: &str,
    attempts: u8,
) -> Result<()> {
    if attempts >= MAX_ASK_ATTEMPTS {
        PendingAction::cancel_all(&state.db, user.id).await?;
        return reply(state, user, &i18n::too_many_tries(lang)).await;
    }

    PendingAction::create(
        &state.db,
        user.business_id,
        user.id,
        &PendingState::NeedInfo {
            attempts: attempts + 1,
        },
        question,
        transcript,
    )
    .await?;
    reply(state, user, question).await
}

/// Read the action back with Yes and No buttons. Nothing is saved until the user agrees.
async fn ask_confirmation(
    state: &AppState,
    user: &User,
    lang: Lang,
    action: Action,
    transcript: &str,
) -> Result<()> {
    let body = i18n::confirm_text(lang, &action);
    PendingAction::create(
        &state.db,
        user.business_id,
        user.id,
        &PendingState::Confirm { action },
        &body,
        transcript,
    )
    .await?;

    let message = format!("{body}\n\n{}", i18n::say_yes_no(lang));
    state
        .whatsapp
        .send_confirm_buttons(
            &user.phone,
            &message,
            i18n::yes_label(lang),
            i18n::no_label(lang),
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlp::{Period, TxKind};

    #[test]
    fn only_owners_may_use_money_and_staff_commands() {
        let record = Command::RecordTransaction {
            kind: TxKind::Sale,
            amount: Some(100.0),
            on_credit: false,
            party_name: None,
            description: None,
        };
        assert!(owner_only(&record));
        assert!(owner_only(&Command::QueryBalance));
        assert!(owner_only(&Command::QuerySummary {
            period: Some(Period::Today)
        }));
        assert!(owner_only(&Command::AddEmployee {
            name: None,
            phone: None
        }));

        assert!(!owner_only(&Command::Help));
        assert!(!owner_only(&Command::CompleteTask {
            task_hint: None,
            assignee_name: None
        }));
        assert!(!owner_only(&Command::QueryTasks {
            assignee_name: None
        }));
        assert!(!owner_only(&Command::SetLanguage { language: None }));
    }

    #[test]
    fn long_transcripts_keep_their_end() {
        let short = "a".repeat(100);
        assert_eq!(keep_tail(&short), short);

        let long = format!("{}END", "x".repeat(3000));
        let kept = keep_tail(&long);
        assert_eq!(kept.chars().count(), MAX_TRANSCRIPT_KEEP);
        assert!(kept.ends_with("END"));
    }
}