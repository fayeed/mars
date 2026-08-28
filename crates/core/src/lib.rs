use std::fmt;

use serde::{Deserialize, Serialize};

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
