use chrono::NaiveDate;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::models::{business::is_supported_language, user::normalize_phone};

/// Largest amount we accept in one entry (PKR). Guards against mis-heard numbers.
pub const MAX_AMOUNT_PKR: f64 = 100_000_000_000.0;

/// The four kinds of money movement. "On credit" (udhaar) is a flag, not a kind:
///   sale             cash sale, or credit sale when on_credit is true
///   expense          cash expense, or bought on credit when on_credit is true
///   payment_received a customer paid us money they owed
///   payment_made     we paid money we owed a supplier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxKind {
    Sale,
    Expense,
    PaymentReceived,
    PaymentMade,
}

impl TxKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TxKind::Sale => "sale",
            TxKind::Expense => "expense",
            TxKind::PaymentReceived => "payment_received",
            TxKind::PaymentMade => "payment_made",
        }
    }

    pub fn from_db(value: &str) -> Option<TxKind> {
        match value {
            "sale" => Some(TxKind::Sale),
            "expense" => Some(TxKind::Expense),
            "payment_received" => Some(TxKind::PaymentReceived),
            "payment_made" => Some(TxKind::PaymentMade),
            _ => None,
        }
    }

    /// +1 when cash comes in, -1 when cash goes out, 0 when no cash moves (credit entries).
    pub fn cash_sign(&self, on_credit: bool) -> i64 {
        match self {
            TxKind::Sale => {
                if on_credit {
                    0
                } else {
                    1
                }
            }
            TxKind::Expense => {
                if on_credit {
                    0
                } else {
                    -1
                }
            }
            TxKind::PaymentReceived => 1,
            TxKind::PaymentMade => -1,
        }
    }

    /// Effect on the party's balance. Convention: positive means the party owes us,
    /// negative means we owe the party.
    pub fn party_sign(&self, on_credit: bool) -> i64 {
        match self {
            TxKind::Sale => {
                if on_credit {
                    1
                } else {
                    0
                }
            }
            TxKind::Expense => {
                if on_credit {
                    -1
                } else {
                    0
                }
            }
            TxKind::PaymentReceived => -1,
            TxKind::PaymentMade => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Period {
    #[default]
    Today,
    Week,
    Month,
}

/// The single command a voice note boils down to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "intent", rename_all = "snake_case")]
pub enum Command {
    RecordTransaction {
        kind: TxKind,
        #[serde(default, deserialize_with = "de_amount")]
        amount: Option<f64>,
        #[serde(default, deserialize_with = "de_bool")]
        on_credit: bool,
        #[serde(default)]
        party_name: Option<String>,
        #[serde(default)]
        description: Option<String>,
    },
    AssignTask {
        #[serde(default)]
        assignee_name: Option<String>,
        #[serde(default)]
        title: Option<String>,
        /// YYYY-MM-DD
        #[serde(default)]
        due_date: Option<String>,
    },
    CompleteTask {
        #[serde(default)]
        task_hint: Option<String>,
        #[serde(default)]
        assignee_name: Option<String>,
    },
    AddEmployee {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        phone: Option<String>,
    },
    QuerySummary {
        #[serde(default)]
        period: Option<Period>,
    },
    QueryBalance,
    QueryParty {
        #[serde(default)]
        party_name: Option<String>,
    },
    QueryTasks {
        #[serde(default)]
        assignee_name: Option<String>,
    },
    SetLanguage {
        #[serde(default)]
        language: Option<String>,
    },
    Help,
    Unclear {
        #[serde(default)]
        reason: Option<String>,
    },
}

/// What the bot still has to ask the user before it can act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    Amount,
    PartyName,
    Assignee,
    TaskTitle,
    EmployeeName,
    EmployeePhone,
    Language,
}

/// A command plus the language the person was speaking.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCommand {
    pub language: Option<String>,
    pub command: Command,
}

impl ParsedCommand {
    /// Build from the JSON the language model returned. Never fails: unreadable output
    /// becomes `Command::Unclear`, so a bad model answer cannot crash or corrupt anything.
    pub fn from_value(value: Value) -> ParsedCommand {
        let language = value
            .get("spoken_language")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_lowercase())
            .filter(|s| is_supported_language(s));

        let command = match serde_json::from_value::<Command>(value) {
            Ok(command) => command.normalized(),
            Err(e) => Command::Unclear {
                reason: Some(format!("unreadable model output: {e}")),
            },
        };

        ParsedCommand { language, command }
    }

    /// Same as `from_value` but starts from text that may contain stray words or code fences
    /// around the JSON object.
    pub fn from_json_text(text: &str) -> ParsedCommand {
        let unclear = |reason: &str| ParsedCommand {
            language: None,
            command: Command::Unclear {
                reason: Some(reason.to_string()),
            },
        };

        match (text.find('{'), text.rfind('}')) {
            (Some(start), Some(end)) if end > start => {
                match serde_json::from_str::<Value>(&text[start..=end]) {
                    Ok(value) => ParsedCommand::from_value(value),
                    Err(_) => unclear("model output was not valid JSON"),
                }
            }
            _ => unclear("model output contained no JSON"),
        }
    }
}

impl Command {
    /// Trim spoken text, turn empty strings into None, and clean up phone numbers and language codes.
    pub fn normalized(self) -> Command {
        match self {
            Command::RecordTransaction {
                kind,
                amount,
                on_credit,
                party_name,
                description,
            } => Command::RecordTransaction {
                kind,
                amount,
                on_credit,
                party_name: clean(party_name),
                description: clean(description),
            },
            Command::AssignTask {
                assignee_name,
                title,
                due_date,
            } => Command::AssignTask {
                assignee_name: clean(assignee_name),
                title: clean(title),
                due_date: clean(due_date),
            },
            Command::CompleteTask {
                task_hint,
                assignee_name,
            } => Command::CompleteTask {
                task_hint: clean(task_hint),
                assignee_name: clean(assignee_name),
            },
            Command::AddEmployee { name, phone } => Command::AddEmployee {
                name: clean(name),
                phone: clean(phone)
                    .map(|p| normalize_phone(&p))
                    .filter(|p| !p.is_empty()),
            },
            Command::QueryParty { party_name } => Command::QueryParty {
                party_name: clean(party_name),
            },
            Command::QueryTasks { assignee_name } => Command::QueryTasks {
                assignee_name: clean(assignee_name),
            },
            Command::SetLanguage { language } => Command::SetLanguage {
                language: clean(language).map(|l| l.to_lowercase()),
            },
            other => other,
        }
    }

    /// Everything that changes data is read back to the user for a Yes/No tap first.
    pub fn requires_confirmation(&self) -> bool {
        matches!(
            self,
            Command::RecordTransaction { .. }
                | Command::AssignTask { .. }
                | Command::CompleteTask { .. }
                | Command::AddEmployee { .. }
        )
    }

    /// The first piece of information that is missing, if any. The bot asks for it by voice or text.
    pub fn missing_info(&self) -> Option<Missing> {
        match self {
            Command::RecordTransaction {
                kind,
                amount,
                on_credit,
                party_name,
                ..
            } => {
                if (*amount).and_then(rupees_to_paisa).is_none() {
                    return Some(Missing::Amount);
                }
                let needs_party =
                    *on_credit || matches!(kind, TxKind::PaymentReceived | TxKind::PaymentMade);
                if needs_party && party_name.is_none() {
                    return Some(Missing::PartyName);
                }
                None
            }
            Command::AssignTask {
                assignee_name,
                title,
                ..
            } => {
                if assignee_name.is_none() {
                    Some(Missing::Assignee)
                } else if title.is_none() {
                    Some(Missing::TaskTitle)
                } else {
                    None
                }
            }
            Command::CompleteTask {
                task_hint,
                assignee_name,
            } => {
                if task_hint.is_none() && assignee_name.is_none() {
                    Some(Missing::TaskTitle)
                } else {
                    None
                }
            }
            Command::AddEmployee { name, phone } => {
                if name.is_none() {
                    Some(Missing::EmployeeName)
                } else if !phone.as_deref().map(is_valid_pk_mobile).unwrap_or(false) {
                    Some(Missing::EmployeePhone)
                } else {
                    None
                }
            }
            Command::QueryParty { party_name } => {
                if party_name.is_none() {
                    Some(Missing::PartyName)
                } else {
                    None
                }
            }
            Command::SetLanguage { language } => {
                let valid = language
                    .as_deref()
                    .map(is_supported_language)
                    .unwrap_or(false);
                if valid {
                    None
                } else {
                    Some(Missing::Language)
                }
            }
            _ => None,
        }
    }

    /// Amount in paisa for a transaction command (None for anything else or an invalid amount).
    pub fn amount_paisa(&self) -> Option<i64> {
        match self {
            Command::RecordTransaction { amount, .. } => (*amount).and_then(rupees_to_paisa),
            _ => None,
        }
    }

    /// Due date of a task command, if one was spoken and is a valid date.
    pub fn due_date(&self) -> Option<NaiveDate> {
        match self {
            Command::AssignTask { due_date, .. } => due_date
                .as_deref()
                .and_then(|s| NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()),
            _ => None,
        }
    }
}

/// Convert rupees (possibly with decimals) to integer paisa. None for zero, negative,
/// non-finite or absurdly large values.
pub fn rupees_to_paisa(rupees: f64) -> Option<i64> {
    if !rupees.is_finite() || rupees <= 0.0 || rupees > MAX_AMOUNT_PKR {
        return None;
    }
    let paisa = (rupees * 100.0).round() as i64;
    if paisa > 0 {
        Some(paisa)
    } else {
        None
    }
}

/// A valid Pakistani mobile number after normalization: 92 followed by 10 digits.
pub fn is_valid_pk_mobile(raw: &str) -> bool {
    let normalized = normalize_phone(raw);
    normalized.len() == 12 && normalized.starts_with("92")
}

fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Accept 2500, 2500.0, "2500" or "2,500" (models sometimes quote numbers); anything else is None.
fn de_amount<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().replace(',', "").parse::<f64>().ok(),
        _ => None,
    })
}

/// Accept true/false, "true"/"yes", or null (treated as false).
fn de_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(Value::Bool(b)) => b,
        Some(Value::String(s)) => matches!(s.trim().to_lowercase().as_str(), "true" | "yes"),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_credit_sale() {
        let parsed = ParsedCommand::from_json_text(
            r#"{"spoken_language":"ur","intent":"record_transaction","kind":"sale",
                "amount":2500,"on_credit":true,"party_name":" Ali ","description":"cloth"}"#,
        );
        assert_eq!(parsed.language.as_deref(), Some("ur"));
        assert_eq!(
            parsed.command,
            Command::RecordTransaction {
                kind: TxKind::Sale,
                amount: Some(2500.0),
                on_credit: true,
                party_name: Some("Ali".to_string()),
                description: Some("cloth".to_string()),
            }
        );
        assert_eq!(parsed.command.missing_info(), None);
        assert_eq!(parsed.command.amount_paisa(), Some(250_000));
        assert!(parsed.command.requires_confirmation());
    }

    #[test]
    fn parses_task_assignment_with_due_date() {
        let parsed = ParsedCommand::from_json_text(
            r#"Here you go: {"spoken_language":"pa","intent":"assign_task",
                "assignee_name":"Bilal","title":"Deliver cloth to Lahore","due_date":"2026-11-23"}"#,
        );
        assert_eq!(parsed.language.as_deref(), Some("pa"));
        assert_eq!(parsed.command.missing_info(), None);
        assert_eq!(
            parsed.command.due_date(),
            NaiveDate::from_ymd_opt(2026, 11, 23)
        );
    }

    #[test]
    fn accepts_amount_written_as_string() {
        let parsed = ParsedCommand::from_json_text(
            r#"{"intent":"record_transaction","kind":"expense","amount":"1,500"}"#,
        );
        assert_eq!(parsed.command.amount_paisa(), Some(150_000));
    }

    #[test]
    fn null_amount_is_reported_as_missing() {
        let parsed = ParsedCommand::from_json_text(
            r#"{"intent":"record_transaction","kind":"sale","amount":null,"on_credit":null}"#,
        );
        assert_eq!(parsed.command.missing_info(), Some(Missing::Amount));
    }

    #[test]
    fn credit_and_payments_need_a_party() {
        let credit = ParsedCommand::from_json_text(
            r#"{"intent":"record_transaction","kind":"sale","amount":500,"on_credit":true}"#,
        );
        assert_eq!(credit.command.missing_info(), Some(Missing::PartyName));

        let payment = ParsedCommand::from_json_text(
            r#"{"intent":"record_transaction","kind":"payment_received","amount":500}"#,
        );
        assert_eq!(payment.command.missing_info(), Some(Missing::PartyName));

        let cash = ParsedCommand::from_json_text(
            r#"{"intent":"record_transaction","kind":"sale","amount":500}"#,
        );
        assert_eq!(cash.command.missing_info(), None);
    }

    #[test]
    fn unknown_intent_becomes_unclear() {
        let parsed = ParsedCommand::from_json_text(r#"{"intent":"launch_rocket"}"#);
        assert!(matches!(parsed.command, Command::Unclear { .. }));
    }

    #[test]
    fn bad_kind_becomes_unclear() {
        let parsed =
            ParsedCommand::from_json_text(r#"{"intent":"record_transaction","kind":"bogus"}"#);
        assert!(matches!(parsed.command, Command::Unclear { .. }));
    }

    #[test]
    fn garbage_text_becomes_unclear() {
        assert!(matches!(
            ParsedCommand::from_json_text("no json here").command,
            Command::Unclear { .. }
        ));
    }

    #[test]
    fn unit_variants_ignore_extra_fields() {
        let parsed = ParsedCommand::from_json_text(
            r#"{"spoken_language":"xx","intent":"query_balance","amount":null,"title":null}"#,
        );
        assert_eq!(parsed.command, Command::QueryBalance);
        assert_eq!(parsed.language, None);
    }

    #[test]
    fn command_round_trips_through_json() {
        let original = Command::AssignTask {
            assignee_name: Some("Bilal".to_string()),
            title: Some("Count stock".to_string()),
            due_date: Some("2026-11-24".to_string()),
        };
        let value = serde_json::to_value(&original).unwrap();
        let back: Command = serde_json::from_value(value).unwrap();
        assert_eq!(original, back);

        let tx = Command::RecordTransaction {
            kind: TxKind::PaymentMade,
            amount: Some(1200.5),
            on_credit: false,
            party_name: Some("Supplier".to_string()),
            description: None,
        };
        let value = serde_json::to_value(&tx).unwrap();
        let back: Command = serde_json::from_value(value).unwrap();
        assert_eq!(tx, back);
    }

    #[test]
    fn cash_and_party_signs_follow_accounting_rules() {
        // cash sale: cash in, no party effect
        assert_eq!(TxKind::Sale.cash_sign(false), 1);
        assert_eq!(TxKind::Sale.party_sign(false), 0);
        // credit sale: no cash, customer owes us
        assert_eq!(TxKind::Sale.cash_sign(true), 0);
        assert_eq!(TxKind::Sale.party_sign(true), 1);
        // credit purchase: no cash, we owe supplier
        assert_eq!(TxKind::Expense.cash_sign(true), 0);
        assert_eq!(TxKind::Expense.party_sign(true), -1);
        // customer pays us: cash in, they owe less
        assert_eq!(TxKind::PaymentReceived.cash_sign(false), 1);
        assert_eq!(TxKind::PaymentReceived.party_sign(false), -1);
        // we pay supplier: cash out, we owe less
        assert_eq!(TxKind::PaymentMade.cash_sign(false), -1);
        assert_eq!(TxKind::PaymentMade.party_sign(false), 1);
    }

    #[test]
    fn converts_rupees_to_paisa_safely() {
        assert_eq!(rupees_to_paisa(2500.0), Some(250_000));
        assert_eq!(rupees_to_paisa(1500.75), Some(150_075));
        assert_eq!(rupees_to_paisa(0.0), None);
        assert_eq!(rupees_to_paisa(-5.0), None);
        assert_eq!(rupees_to_paisa(f64::NAN), None);
        assert_eq!(rupees_to_paisa(1e15), None);
    }

    #[test]
    fn validates_employee_phone() {
        let good = ParsedCommand::from_json_text(
            r#"{"intent":"add_employee","name":"Bilal","phone":"0300-1234567"}"#,
        );
        assert_eq!(good.command.missing_info(), None);
        assert_eq!(
            good.command,
            Command::AddEmployee {
                name: Some("Bilal".to_string()),
                phone: Some("923001234567".to_string()),
            }
        );

        let bad = ParsedCommand::from_json_text(
            r#"{"intent":"add_employee","name":"Bilal","phone":"123"}"#,
        );
        assert_eq!(bad.command.missing_info(), Some(Missing::EmployeePhone));
    }
}