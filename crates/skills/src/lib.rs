//! Inspected, pinned, and scoped agent skill installation.

mod install;
mod policy;
mod registry;

pub use install::{InspectedSkill, SkillError, SkillInstallation, SkillScope, SkillService};
pub use registry::{RegistryClient, SkillCandidate, SkillFile, SkillSnapshot, SkillsShRegistry};
