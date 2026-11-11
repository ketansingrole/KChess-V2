//! What the core asks of the frontend it runs for, and the answer (`docs/shared-core.md`): the
//! core emits `host:request` as a [`HostAsk`], and the frontend answers with `host.reply(id,
//! reply)`, a [`HostReply`]. Each request needs one [`Capability`] of the client it goes to.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// Something a client can do for the core when it asks, as it says when it connects.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    /// Open a web page in the browser.
    Browser,
    /// Bring its window to the front, and say whether the computer runs on battery.
    Focus,
    /// Seal and open logins itself, for a core without the profile's credentials.
    Secrets,
}

/// One thing the core asks its frontend for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostRequest {
    /// Open this web page in the browser; answered with `null`.
    OpenExternal(String),
    /// Bring the window to the front; answered with `null`.
    Focus,
    /// Whether the computer runs on battery; answered with a boolean.
    OnBattery,
    /// Whether the frontend can seal logins; answered with a boolean.
    SecretsAvailable,
    /// Seal this login; answered with the sealed text.
    SecretsEncrypt(String),
    /// Open this sealed login; answered with the plain text.
    SecretsDecrypt(String),
}

impl HostRequest {
    /// The kind it crosses as.
    pub fn kind(&self) -> &'static str {
        match self {
            HostRequest::OpenExternal(_) => "openExternal",
            HostRequest::Focus => "focus",
            HostRequest::OnBattery => "onBattery",
            HostRequest::SecretsAvailable => "secrets.available",
            HostRequest::SecretsEncrypt(_) => "secrets.encrypt",
            HostRequest::SecretsDecrypt(_) => "secrets.decrypt",
        }
    }

    /// What a client must be able to do to answer it.
    pub fn needs(&self) -> Capability {
        match self {
            HostRequest::OpenExternal(_) => Capability::Browser,
            HostRequest::Focus | HostRequest::OnBattery => Capability::Focus,
            HostRequest::SecretsAvailable
            | HostRequest::SecretsEncrypt(_)
            | HostRequest::SecretsDecrypt(_) => Capability::Secrets,
        }
    }

    fn payload(&self) -> Value {
        match self {
            HostRequest::OpenExternal(text)
            | HostRequest::SecretsEncrypt(text)
            | HostRequest::SecretsDecrypt(text) => Value::String(text.clone()),
            HostRequest::Focus | HostRequest::OnBattery | HostRequest::SecretsAvailable => {
                Value::Null
            }
        }
    }

    fn from_parts(kind: &str, payload: Value) -> Option<HostRequest> {
        let text = || payload.as_str().map(str::to_string);
        Some(match kind {
            "openExternal" => HostRequest::OpenExternal(text()?),
            "focus" => HostRequest::Focus,
            "onBattery" => HostRequest::OnBattery,
            "secrets.available" => HostRequest::SecretsAvailable,
            "secrets.encrypt" => HostRequest::SecretsEncrypt(text()?),
            "secrets.decrypt" => HostRequest::SecretsDecrypt(text()?),
            _ => return None,
        })
    }
}

/// A request as the `host:request` event carries it: `{ id, kind, payload }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostAsk {
    /// What the reply names.
    pub id: u64,
    pub request: HostRequest,
}

#[derive(Deserialize)]
struct WireAsk {
    id: u64,
    kind: String,
    #[serde(default)]
    payload: Value,
}

impl Serialize for HostAsk {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_value().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for HostAsk {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<HostAsk, D::Error> {
        let wire = WireAsk::deserialize(deserializer)?;
        let request = HostRequest::from_parts(&wire.kind, wire.payload).ok_or_else(|| {
            serde::de::Error::custom(format!("no host request {} of that shape", wire.kind))
        })?;
        Ok(HostAsk {
            id: wire.id,
            request,
        })
    }
}

impl HostAsk {
    /// The event payload it crosses as.
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "kind": self.request.kind(),
            "payload": self.request.payload(),
        })
    }
}

/// A frontend's answer to a [`HostRequest`]: `{ "value": … }` or `{ "error": "…" }`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum HostReply {
    Value(Value),
    Error(String),
}

impl HostReply {
    /// The answer to a request that answers nothing.
    pub fn done() -> HostReply {
        HostReply::Value(Value::Null)
    }

    pub fn into_result(self) -> Result<Value, String> {
        match self {
            HostReply::Value(value) => Ok(value),
            HostReply::Error(message) => Err(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Requests and replies cross in the shapes every earlier build of this release sends.
    #[test]
    fn requests_and_replies_keep_their_wire_shape() {
        let open = HostAsk {
            id: 3,
            request: HostRequest::OpenExternal("https://lichess.org/".into()),
        };
        assert_eq!(
            open.to_value(),
            json!({ "id": 3, "kind": "openExternal", "payload": "https://lichess.org/" })
        );
        let focus: HostAsk =
            serde_json::from_value(json!({ "id": 4, "kind": "focus", "payload": null }))
                .expect("focus");
        assert_eq!(focus.request, HostRequest::Focus);
        for request in [
            HostRequest::OpenExternal("x".into()),
            HostRequest::Focus,
            HostRequest::OnBattery,
            HostRequest::SecretsAvailable,
            HostRequest::SecretsEncrypt("plain".into()),
            HostRequest::SecretsDecrypt("sealed".into()),
        ] {
            let ask = HostAsk { id: 1, request };
            assert_eq!(
                serde_json::from_value::<HostAsk>(ask.to_value()).expect("reads back"),
                ask
            );
        }
        assert!(
            serde_json::from_value::<HostAsk>(json!({ "id": 1, "kind": "openExternal" })).is_err(),
            "a page to open names it"
        );
        assert_eq!(
            serde_json::to_value(HostReply::done()).expect("json"),
            json!({ "value": null })
        );
        assert_eq!(
            serde_json::to_value(HostReply::Error("No.".into())).expect("json"),
            json!({ "error": "No." })
        );
        assert_eq!(
            serde_json::from_value::<HostReply>(json!({ "value": true })).expect("reply"),
            HostReply::Value(json!(true))
        );
    }

    #[test]
    fn capabilities_cross_by_name() {
        assert_eq!(
            serde_json::to_value([Capability::Browser, Capability::Focus]).expect("json"),
            json!(["browser", "focus"])
        );
        assert_eq!(
            HostRequest::SecretsDecrypt(String::new()).needs(),
            Capability::Secrets
        );
        assert_eq!(HostRequest::OnBattery.needs(), Capability::Focus);
    }
}
