use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::protocol::{Command, CommandRequest, CommandResponse, ProtocolError, ResponsePayload};

pub mod jobs;
pub mod protocol;

const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::new(0, 1, 0);

/// The stable boundary between the desktop shell and the core engine.
#[derive(Debug, Default)]
pub struct Core;

impl Core {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    #[must_use]
    pub const fn protocol_version(&self) -> ProtocolVersion {
        PROTOCOL_VERSION
    }

    #[must_use]
    pub fn execute(&self, request: CommandRequest) -> CommandResponse {
        if request.protocol_version != PROTOCOL_VERSION {
            return CommandResponse::Failure {
                protocol_version: PROTOCOL_VERSION,
                correlation_id: request.correlation_id,
                error: ProtocolError {
                    code: "protocol.version_mismatch".into(),
                    message: format!(
                        "desktop protocol {} is incompatible with core protocol {}",
                        request.protocol_version, PROTOCOL_VERSION
                    ),
                    retryable: false,
                    details: Some(json!({ "supported": PROTOCOL_VERSION })),
                },
            };
        }

        match request.command {
            Command::CoreInfo {} => CommandResponse::Success {
                protocol_version: PROTOCOL_VERSION,
                correlation_id: request.correlation_id,
                result: ResponsePayload::CoreInfo {
                    protocol_version: PROTOCOL_VERSION,
                },
            },
        }
    }
}

/// A transport-safe semantic version for the core protocol.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl ProtocolVersion {
    #[must_use]
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::{Core, ProtocolVersion};

    #[test]
    fn core_reports_its_semantic_protocol_version() {
        let core = Core::new();

        assert_eq!(core.protocol_version(), ProtocolVersion::new(0, 1, 0));
        assert_eq!(core.protocol_version().to_string(), "0.1.0");
    }
}
