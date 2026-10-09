use serde::{Deserialize, Serialize};

use crate::install::SkillError;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCandidate {
    pub name: String,
    pub slug: String,
    pub source: String,
    pub installs: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkillFile {
    pub path: String,
    pub contents: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkillSnapshot {
    pub files: Vec<SkillFile>,
    pub hash: String,
}

pub trait RegistryClient: Send + Sync {
    fn search(&self, query: &str) -> Result<Vec<SkillCandidate>, SkillError>;
    fn download(&self, source: &str, slug: &str) -> Result<SkillSnapshot, SkillError>;
}

#[derive(Clone)]
pub struct SkillsShRegistry {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl SkillsShRegistry {
    pub fn new() -> Result<Self, SkillError> {
        Ok(Self {
            base_url: "https://skills.sh".into(),
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()?,
        })
    }
}

#[derive(Deserialize)]
struct SearchResponse {
    skills: Vec<SearchRecord>,
}

#[derive(Deserialize)]
struct SearchRecord {
    /// `owner/repository/skill`.
    id: String,
    /// The path segment the download endpoint wants. Usually the name, but
    /// not necessarily, so it is read rather than assumed.
    #[serde(rename = "skillId", default)]
    skill_id: Option<String>,
    name: String,
    source: String,
    #[serde(default)]
    installs: u64,
}

impl RegistryClient for SkillsShRegistry {
    fn search(&self, query: &str) -> Result<Vec<SkillCandidate>, SkillError> {
        if query.trim().len() < 2 {
            return Ok(Vec::new());
        }
        let response = self
            .client
            .get(format!("{}/api/search", self.base_url))
            .query(&[("q", query.trim()), ("limit", "20")])
            .send()?
            .error_for_status()?
            .json::<SearchResponse>()?;
        Ok(response
            .skills
            .into_iter()
            .map(|record| {
                let slug = record
                    .skill_id
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(|| {
                        record
                            .id
                            .rsplit('/')
                            .next()
                            .unwrap_or(record.name.as_str())
                            .to_owned()
                    });
                SkillCandidate {
                    name: record.name,
                    slug,
                    source: record.source,
                    installs: record.installs,
                }
            })
            .collect())
    }

    fn download(&self, source: &str, slug: &str) -> Result<SkillSnapshot, SkillError> {
        let (owner, repository) = parse_source(source)?;
        Ok(self
            .client
            .get(format!(
                "{}/api/download/{owner}/{repository}/{slug}",
                self.base_url
            ))
            .send()?
            .error_for_status()?
            .json::<SkillSnapshot>()?)
    }
}

fn parse_source(source: &str) -> Result<(&str, &str), SkillError> {
    let mut parts = source.split('/');
    let owner = parts.next().unwrap_or_default();
    let repository = parts.next().unwrap_or_default();
    if owner.is_empty() || repository.is_empty() || parts.next().is_some() {
        return Err(SkillError::InvalidSource(source.to_owned()));
    }
    Ok((owner, repository))
}
