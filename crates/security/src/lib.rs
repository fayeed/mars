//! Repository trust and capability policy types.

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
