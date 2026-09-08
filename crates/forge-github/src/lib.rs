use mars_forge::{
    CheckRun, CreatePullRequest, ForgeError, ForgeProvider, ForgeRepository, ForgeUser, Issue,
    PullRequest, ReviewComment,
};
use reqwest::Method;
use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Arc;

pub struct GitHubProvider {
    repository: ForgeRepository,
    api_base: String,
    transport: Arc<dyn GitHubTransport>,
}

pub trait GitHubTransport: Send + Sync {
    fn send(&self, method: Method, url: &str, body: Option<Value>) -> Result<Value, ForgeError>;
}

struct ReqwestTransport {
    client: Client,
}

impl GitHubProvider {
    pub fn new(
        repository: ForgeRepository,
        api_base: impl Into<String>,
        token: &str,
    ) -> Result<Self, ForgeError> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("mars-agent-ide/0.1"));
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        let authorization = HeaderValue::from_str(&format!("Bearer {}", token.trim()))
            .map_err(|error| ForgeError::Request(error.to_string()))?;
        headers.insert(AUTHORIZATION, authorization);
        let client = Client::builder()
            .default_headers(headers)
            .build()
            .map_err(|error| ForgeError::Request(error.to_string()))?;
        Ok(Self::with_transport(
            repository,
            api_base,
            Arc::new(ReqwestTransport { client }),
        ))
    }

    #[must_use]
    pub fn with_transport(
        repository: ForgeRepository,
        api_base: impl Into<String>,
        transport: Arc<dyn GitHubTransport>,
    ) -> Self {
        Self {
            repository,
            api_base: api_base.into().trim_end_matches('/').into(),
            transport,
        }
    }

    fn endpoint(&self, suffix: &str) -> String {
        format!(
            "{}/repos/{}/{}{}",
            self.api_base, self.repository.owner, self.repository.name, suffix
        )
    }

    fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        suffix: &str,
        body: Option<Value>,
    ) -> Result<T, ForgeError> {
        serde_json::from_value(self.transport.send(method, &self.endpoint(suffix), body)?)
            .map_err(|error| ForgeError::InvalidResponse(error.to_string()))
    }

    fn request_url<T: DeserializeOwned>(
        &self,
        method: Method,
        url: &str,
        body: Option<Value>,
    ) -> Result<T, ForgeError> {
        serde_json::from_value(self.transport.send(method, url, body)?)
            .map_err(|error| ForgeError::InvalidResponse(error.to_string()))
    }
}

impl GitHubTransport for ReqwestTransport {
    fn send(&self, method: Method, url: &str, body: Option<Value>) -> Result<Value, ForgeError> {
        let mut request = self.client.request(method, url);
        if let Some(body) = body {
            request = request.json(&body);
        }
        decode_response(
            request
                .send()
                .map_err(|error| ForgeError::Request(error.to_string()))?,
        )
    }
}

impl ForgeProvider for GitHubProvider {
    fn repository(&self) -> &ForgeRepository {
        &self.repository
    }

    fn current_user(&self) -> Result<ForgeUser, ForgeError> {
        let user: GitHubCurrentUser =
            self.request_url(Method::GET, &format!("{}/user", self.api_base), None)?;
        Ok(ForgeUser {
            login: user.login,
            url: user.html_url,
        })
    }

    fn get_issue(&self, number: u64) -> Result<Issue, ForgeError> {
        let raw: GitHubIssue = self.request(Method::GET, &format!("/issues/{number}"), None)?;
        Ok(raw.into())
    }

    fn request_reviewers(
        &self,
        number: u64,
        reviewers: &[String],
    ) -> Result<Vec<String>, ForgeError> {
        if reviewers.is_empty() {
            return Ok(Vec::new());
        }
        let body = serde_json::json!({ "reviewers": reviewers });
        let requested: GitHubPull = self.request(
            Method::POST,
            &format!("/pulls/{number}/requested_reviewers"),
            Some(body),
        )?;
        Ok(requested
            .requested_reviewers
            .into_iter()
            .map(|user| user.login)
            .collect())
    }

    fn create_pull_request(&self, input: &CreatePullRequest) -> Result<PullRequest, ForgeError> {
        let body = serde_json::to_value(input)
            .map_err(|error| ForgeError::InvalidResponse(error.to_string()))?;
        let created: Result<GitHubPull, ForgeError> =
            self.request(Method::POST, "/pulls", Some(body));
        match created {
            Ok(raw) => Ok(raw.into()),
            Err(ForgeError::Http { status: 422, .. }) => {
                let mut url = reqwest::Url::parse(&self.endpoint("/pulls"))
                    .map_err(|error| ForgeError::Request(error.to_string()))?;
                url.query_pairs_mut()
                    .append_pair("state", "open")
                    .append_pair("head", &format!("{}:{}", self.repository.owner, input.head))
                    .append_pair("base", &input.base)
                    .append_pair("per_page", "100");
                let existing: Vec<GitHubPull> =
                    self.request_url(Method::GET, url.as_str(), None)?;
                existing
                    .into_iter()
                    .find(|pull| pull.head.name == input.head && pull.base.name == input.base)
                    .map(Into::into)
                    .ok_or_else(|| ForgeError::Http {
                        status: 422,
                        message: "GitHub rejected PR creation and no matching open PR exists"
                            .into(),
                    })
            }
            Err(error) => Err(error),
        }
    }

    fn get_pull_request(&self, number: u64) -> Result<PullRequest, ForgeError> {
        let raw: GitHubPull = self.request(Method::GET, &format!("/pulls/{number}"), None)?;
        Ok(raw.into())
    }

    fn list_review_comments(&self, number: u64) -> Result<Vec<ReviewComment>, ForgeError> {
        let mut comments = Vec::new();
        for page in 1..=100 {
            let raw: Vec<GitHubComment> = self.request(
                Method::GET,
                &format!("/pulls/{number}/comments?per_page=100&page={page}"),
                None,
            )?;
            let complete = raw.len() < 100;
            comments.extend(raw.into_iter().map(Into::into));
            if complete {
                return Ok(comments);
            }
        }
        Err(ForgeError::InvalidResponse(
            "GitHub review comment pagination exceeded 100 pages".into(),
        ))
    }

    fn list_checks(&self, revision: &str) -> Result<Vec<CheckRun>, ForgeError> {
        let mut checks = Vec::new();
        for page in 1..=100 {
            let raw: GitHubChecks = self.request(
                Method::GET,
                &format!("/commits/{revision}/check-runs?per_page=100&page={page}"),
                None,
            )?;
            let complete = raw.check_runs.len() < 100;
            checks.extend(raw.check_runs.into_iter().map(Into::into));
            if complete {
                return Ok(checks);
            }
        }
        Err(ForgeError::InvalidResponse(
            "GitHub check pagination exceeded 100 pages".into(),
        ))
    }
}

#[derive(Deserialize)]
struct User {
    login: String,
}

#[derive(Deserialize)]
struct GitHubCurrentUser {
    login: String,
    html_url: String,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

#[derive(Deserialize)]
struct GitRef {
    #[serde(rename = "ref")]
    name: String,
    sha: String,
}

#[derive(Deserialize)]
struct GitHubIssue {
    id: u64,
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    html_url: String,
    user: User,
    labels: Vec<Label>,
    updated_at: String,
}

impl From<GitHubIssue> for Issue {
    fn from(value: GitHubIssue) -> Self {
        Self {
            provider_id: value.id.to_string(),
            number: value.number,
            title: value.title,
            body: value.body.unwrap_or_default(),
            state: value.state,
            url: value.html_url,
            author: value.user.login,
            labels: value.labels.into_iter().map(|label| label.name).collect(),
            updated_at: value.updated_at,
        }
    }
}

#[derive(Deserialize)]
struct GitHubLogin {
    login: String,
}

#[derive(Deserialize)]
struct GitHubPull {
    id: u64,
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    html_url: String,
    head: GitRef,
    base: GitRef,
    draft: bool,
    mergeable: Option<bool>,
    updated_at: String,
    /// Absent from most pull payloads, so it defaults rather than failing the
    /// whole response when GitHub does not send it.
    #[serde(default)]
    requested_reviewers: Vec<GitHubLogin>,
}

impl From<GitHubPull> for PullRequest {
    fn from(value: GitHubPull) -> Self {
        Self {
            provider_id: value.id.to_string(),
            number: value.number,
            title: value.title,
            body: value.body.unwrap_or_default(),
            state: value.state,
            url: value.html_url,
            head: value.head.name,
            base: value.base.name,
            draft: value.draft,
            mergeable: value.mergeable,
            head_sha: value.head.sha,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Deserialize)]
struct GitHubComment {
    id: u64,
    user: User,
    body: String,
    path: Option<String>,
    line: Option<u64>,
    html_url: String,
    created_at: String,
}

impl From<GitHubComment> for ReviewComment {
    fn from(value: GitHubComment) -> Self {
        Self {
            provider_id: value.id.to_string(),
            author: value.user.login,
            body: value.body,
            path: value.path,
            line: value.line,
            url: value.html_url,
            created_at: value.created_at,
        }
    }
}

#[derive(Deserialize)]
struct GitHubChecks {
    check_runs: Vec<GitHubCheck>,
}

#[derive(Deserialize)]
struct GitHubCheck {
    id: u64,
    name: String,
    status: String,
    conclusion: Option<String>,
    html_url: String,
    started_at: Option<String>,
    completed_at: Option<String>,
}

impl From<GitHubCheck> for CheckRun {
    fn from(value: GitHubCheck) -> Self {
        Self {
            provider_id: value.id.to_string(),
            name: value.name,
            status: value.status,
            conclusion: value.conclusion,
            url: value.html_url,
            started_at: value.started_at,
            completed_at: value.completed_at,
        }
    }
}

fn decode_response(response: Response) -> Result<Value, ForgeError> {
    let status = response.status();
    if !status.is_success() {
        let message = response
            .text()
            .map(|value| truncate(&value, 1_000))
            .unwrap_or_else(|error| error.to_string());
        return Err(ForgeError::Http {
            status: status.as_u16(),
            message,
        });
    }
    response
        .json()
        .map_err(|error| ForgeError::InvalidResponse(error.to_string()))
}

fn truncate(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}
