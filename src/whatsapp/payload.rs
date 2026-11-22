use serde::Deserialize;

// ---------- Raw shapes of Meta's webhook JSON (every field optional so odd payloads never crash us) ----------

#[derive(Debug, Default, Deserialize)]
pub struct WebhookPayload {
    #[serde(default)]
    pub entry: Vec<Entry>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub changes: Vec<Change>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Change {
    #[serde(default)]
    pub value: ChangeValue,
}

#[derive(Debug, Default, Deserialize)]
pub struct ChangeValue {
    #[serde(default)]
    pub contacts: Vec<Contact>,
    #[serde(default)]
    pub messages: Vec<RawMessage>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Contact {
    #[serde(default)]
    pub wa_id: String,
    #[serde(default)]
    pub profile: Profile,
}

#[derive(Debug, Default, Deserialize)]
pub struct Profile {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawMessage {
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub text: Option<TextBody>,
    #[serde(default)]
    pub audio: Option<MediaBody>,
    #[serde(default)]
    pub interactive: Option<Interactive>,
}

#[derive(Debug, Default, Deserialize)]
pub struct TextBody {
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct MediaBody {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub mime_type: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct Interactive {
    #[serde(default)]
    pub button_reply: Option<Reply>,
    #[serde(default)]
    pub list_reply: Option<Reply>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Reply {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
}

// ---------- Clean shapes the rest of the app works with ----------

#[derive(Debug, Clone, PartialEq)]
pub enum MessageContent {
    Audio { media_id: String, mime_type: String },
    Text(String),
    ButtonReply { id: String, title: String },
    Unsupported(String),
}

#[derive(Debug, Clone)]
pub struct IncomingMessage {
    /// WhatsApp message id (wamid...), used for de-duplication and read receipts.
    pub id: String,
    /// Sender's WhatsApp number, digits only with country code.
    pub from: String,
    /// Name from the sender's WhatsApp profile, if provided.
    pub sender_name: Option<String>,
    pub content: MessageContent,
}

impl WebhookPayload {
    /// Flatten Meta's nested payload into a simple list of messages.
    /// Status updates (sent/delivered/read) carry no messages and yield nothing.
    pub fn messages(&self) -> Vec<IncomingMessage> {
        let mut out = Vec::new();

        for entry in &self.entry {
            for change in &entry.changes {
                let value = &change.value;
                for raw in &value.messages {
                    if raw.id.is_empty() || raw.from.is_empty() {
                        continue;
                    }

                    let sender_name = value
                        .contacts
                        .iter()
                        .find(|c| c.wa_id == raw.from)
                        .map(|c| c.profile.name.trim().to_string())
                        .filter(|n| !n.is_empty());

                    out.push(IncomingMessage {
                        id: raw.id.clone(),
                        from: raw.from.clone(),
                        sender_name,
                        content: content_of(raw),
                    });
                }
            }
        }

        out
    }
}

fn content_of(raw: &RawMessage) -> MessageContent {
    match raw.kind.as_str() {
        "audio" => match &raw.audio {
            Some(a) if !a.id.is_empty() => MessageContent::Audio {
                media_id: a.id.clone(),
                mime_type: a.mime_type.clone(),
            },
            _ => MessageContent::Unsupported("audio".to_string()),
        },
        "text" => match &raw.text {
            Some(t) => MessageContent::Text(t.body.clone()),
            None => MessageContent::Unsupported("text".to_string()),
        },
        "interactive" => {
            let reply = raw.interactive.as_ref().and_then(|i| {
                i.button_reply.as_ref().or(i.list_reply.as_ref())
            });
            match reply {
                Some(r) => MessageContent::ButtonReply {
                    id: r.id.clone(),
                    title: r.title.clone(),
                },
                None => MessageContent::Unsupported("interactive".to_string()),
            }
        }
        other => MessageContent::Unsupported(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOICE_NOTE: &str = r#"{
      "object": "whatsapp_business_account",
      "entry": [{
        "id": "111",
        "changes": [{
          "field": "messages",
          "value": {
            "messaging_product": "whatsapp",
            "contacts": [{ "profile": { "name": "Ahmed Khan" }, "wa_id": "923001234567" }],
            "messages": [{
              "from": "923001234567",
              "id": "wamid.ABC",
              "timestamp": "1700000000",
              "type": "audio",
              "audio": { "id": "media-99", "mime_type": "audio/ogg; codecs=opus", "voice": true }
            }]
          }
        }]
      }]
    }"#;

    const BUTTON_TAP: &str = r#"{
      "entry": [{ "changes": [{ "value": {
        "messages": [{
          "from": "923001234567", "id": "wamid.BTN", "type": "interactive",
          "interactive": { "type": "button_reply",
                           "button_reply": { "id": "confirm_yes", "title": "Yes" } }
        }]
      }}]}]
    }"#;

    const STATUS_ONLY: &str = r#"{
      "entry": [{ "changes": [{ "value": {
        "statuses": [{ "id": "wamid.X", "status": "delivered" }]
      }}]}]
    }"#;

    #[test]
    fn parses_voice_note() {
        let payload: WebhookPayload = serde_json::from_str(VOICE_NOTE).unwrap();
        let msgs = payload.messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, "wamid.ABC");
        assert_eq!(msgs[0].from, "923001234567");
        assert_eq!(msgs[0].sender_name.as_deref(), Some("Ahmed Khan"));
        assert_eq!(
            msgs[0].content,
            MessageContent::Audio {
                media_id: "media-99".to_string(),
                mime_type: "audio/ogg; codecs=opus".to_string()
            }
        );
    }

    #[test]
    fn parses_button_reply() {
        let payload: WebhookPayload = serde_json::from_str(BUTTON_TAP).unwrap();
        let msgs = payload.messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0].content,
            MessageContent::ButtonReply {
                id: "confirm_yes".to_string(),
                title: "Yes".to_string()
            }
        );
    }

    #[test]
    fn status_updates_yield_no_messages() {
        let payload: WebhookPayload = serde_json::from_str(STATUS_ONLY).unwrap();
        assert!(payload.messages().is_empty());
    }

    #[test]
    fn empty_object_is_fine() {
        let payload: WebhookPayload = serde_json::from_str("{}").unwrap();
        assert!(payload.messages().is_empty());
    }
}