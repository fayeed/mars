//! The activity record: the append-only log every other execution-core service
//! writes to and the entire UI renders from.
//!
//! Three properties are load-bearing:
//!
//! * **Append-only.** Events are immutable once written, enforced by a database
//!   trigger rather than by convention. Rows leave only with their task.
//! * **Monotonic per task.** A sequence is assigned inside the insert, so no
//!   caller can choose its own ordering or leave a gap.
//! * **Content-addressed payloads.** Command output, diffs, and test results are
//!   stored by digest, keeping event rows pageable and repeated payloads free.
//!
//! Because the log holds everything, reload, crash recovery, and export are the
//! same operation at different times: replay it.

pub mod audit;
pub mod payloads;
pub mod record;
pub mod recovery;
pub mod replay;
pub mod runs;

use std::path::PathBuf;

use thiserror::Error;

pub use audit::{MAX_EVENTS as AUDIT_MAX_EVENTS, MAX_PAYLOAD_BYTES as AUDIT_MAX_PAYLOAD_BYTES};
pub use payloads::{PayloadStore, StoredPayload};
pub use record::{
    ActivityEntry, ActivityEvent, ActivityKind, ActivityRecord, ActivityWindow, Actor, Attachment,
};
pub use recovery::{
    InFlightItem, InterruptedRun, RecoveryAction, RecoveryOption, RecoveryService, WrittenFile,
};
pub use replay::{
    CommandRun, DecidedPermission, FileTouch, PendingPermission, ReplayedRun, ReplayedRunState,
    TaskProjection, TestOutcome, WorktreeView, project, replay,
};
pub use runs::{NewRun, RunRecord, RunRole, RunStatus, RunStore};

#[derive(Debug, Error)]
pub enum ActivityError {
    #[error(transparent)]
    Storage(#[from] mars_storage::StorageError),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("failed to encode or decode activity JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to format an activity timestamp: {0}")]
    Timestamp(#[from] time::error::Format),
    #[error("failed to read or write activity payload at {path}: {source}")]
    PayloadIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("stored payload {digest} does not match its digest")]
    PayloadCorrupt { digest: String },
    #[error("run `{0}` does not exist")]
    RunNotFound(String),
    #[error("run `{0}` has already finished")]
    RunAlreadyFinished(String),
    #[error("stored activity kind is invalid: {0}")]
    InvalidStoredKind(String),
    #[error("stored activity actor is invalid: {0}")]
    InvalidStoredActor(String),
    #[error("stored run status is invalid: {0}")]
    InvalidStoredStatus(String),
    #[error("stored run role is invalid: {0}")]
    InvalidStoredRole(String),
    /// A record too large to render as one document. Refused loudly rather than
    /// exported partially: an audit is whole or it is not an audit.
    #[error("{0}")]
    TooLargeToRender(String),
}
