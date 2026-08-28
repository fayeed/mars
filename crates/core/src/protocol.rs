use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ProtocolVersion;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CommandRequest {
    pub protocol_version: ProtocolVersion,
    pub correlation_id: String,
    pub command: Command,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "type")]
pub enum Command {
    CoreInfo {},
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "status")]
pub enum CommandResponse {
    Success {
        protocol_version: ProtocolVersion,
        correlation_id: String,
        result: ResponsePayload,
    },
    Failure {
        protocol_version: ProtocolVersion,
        correlation_id: String,
        error: ProtocolError,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "type")]
pub enum ResponsePayload {
    CoreInfo { protocol_version: ProtocolVersion },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProtocolError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CoreEvent {
    pub protocol_version: ProtocolVersion,
    pub sequence: u64,
    pub event: EventPayload,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EventPayload {
    #[serde(rename = "type")]
    pub event_type: String,
    pub payload: Value,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl EventPayload {
    pub fn new(
        event_type: impl Into<String>,
        payload: Value,
        extensions: BTreeMap<String, Value>,
    ) -> Result<Self, ReservedExtensionKey> {
        for reserved in ["type", "payload"] {
            if extensions.contains_key(reserved) {
                return Err(ReservedExtensionKey(reserved));
            }
        }

        Ok(Self {
            event_type: event_type.into(),
            payload,
            extensions,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReservedExtensionKey(&'static str);

impl fmt::Display for ReservedExtensionKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "event extension uses reserved key `{}`", self.0)
    }
}

impl Error for ReservedExtensionKey {}
