//! Inspected, pinned, and scoped agent skill installation.

mod install;
mod local;
mod policy;
mod registry;

pub use install::{
    InspectedSkill, SkillError, SkillInstallation, SkillScope, SkillService, SkillUpdate,
    front_matter_field, registry_digest,
};
pub use local::{LocalOrigin, LocalSkill, discover_local};
pub use policy::AdapterPolicy;
pub use registry::{RegistryClient, SkillCandidate, SkillFile, SkillSnapshot, SkillsShRegistry};
