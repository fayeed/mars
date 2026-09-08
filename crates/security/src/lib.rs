//! Repository trust and capability policy types.

pub mod broker;
pub mod classify;
pub mod ignore_policy;
pub mod permissions;
pub mod redaction;

pub use broker::{
    BrokerDecision, BrokerOutcome, BrokerRequest, CapabilityCeiling, CommandCeiling, GrantView,
    PendingRequest, PermissionBroker, PermissionTicket, SideEffect, glob_match,
};
pub use classify::{CommandAnalysis, CommandClass, classify_command};
pub use ignore_policy::{is_default_ignored_directory, is_sensitive_filename};
pub use redaction::{detect_secret, redact};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrustDecision {
    RequireConfirmation,
    Trust,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RepositoryCapability {
    ReadFiles,
    IndexCode,
    ObserveGit,
}

pub const TRUSTED_REPOSITORY_CAPABILITIES: [RepositoryCapability; 3] = [
    RepositoryCapability::ReadFiles,
    RepositoryCapability::IndexCode,
    RepositoryCapability::ObserveGit,
];
