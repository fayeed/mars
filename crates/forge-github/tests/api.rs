use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use mars_forge::{CreatePullRequest, ForgeError, ForgeProvider, ForgeRepository};
use mars_forge_github::{GitHubProvider, GitHubTransport};
use reqwest::Method;
use serde_json::{Value, json};

#[derive(Default)]
struct FakeTransport {
    responses: Mutex<VecDeque<Result<Value, ForgeError>>>,
    requests: Mutex<Vec<(Method, String, Option<Value>)>>,
}

impl FakeTransport {
    fn with_responses(responses: Vec<Value>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl GitHubTransport for FakeTransport {
    fn send(&self, method: Method, url: &str, body: Option<Value>) -> Result<Value, ForgeError> {
        self.requests
            .lock()
            .expect("requests should lock")
            .push((method, url.into(), body));
        self.responses
            .lock()
            .expect("responses should lock")
            .pop_front()
            .ok_or_else(|| ForgeError::InvalidResponse("missing fake response".into()))?
    }
}

#[test]
fn recovers_a_retry_after_github_reports_an_existing_pull_request() {
    let transport = Arc::new(FakeTransport {
        responses: Mutex::new(VecDeque::from([
            Err(ForgeError::Http {
                status: 422,
                message: "A pull request already exists".into(),
            }),
            Ok(json!([pull_json()])),
        ])),
        requests: Mutex::new(Vec::new()),
    });
    let provider = GitHubProvider::with_transport(
        ForgeRepository {
            host: "github.test".into(),
            owner: "acme".into(),
            name: "mars".into(),
        },
        "https://api.github.test",
        transport.clone(),
    );
    let recovered = provider
        .create_pull_request(&CreatePullRequest {
            title: "Fix sync".into(),
            body: "Closes #7".into(),
            head: "task/sync".into(),
            base: "main".into(),
            draft: false,
        })
        .expect("matching existing PR should recover the retry");
    assert_eq!(recovered.number, 9);
    let requests = transport.requests.lock().expect("requests should unlock");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].0, Method::POST);
    assert_eq!(requests[1].0, Method::GET);
    assert!(requests[1].1.contains("head=acme%3Atask%2Fsync"));
    assert!(requests[1].1.contains("base=main"));
}

#[test]
fn github_api_normalizes_issue_pr_comments_and_checks() {
    let transport = Arc::new(FakeTransport::with_responses(vec![
        json!({ "login": "faye", "html_url": "https://github.test/faye" }),
        json!({
            "id": 101, "number": 7, "title": "Fix sync", "body": "Keep local state",
            "state": "open", "html_url": "https://github.test/acme/mars/issues/7",
            "user": { "login": "faye" }, "labels": [{ "name": "bug" }],
            "updated_at": "2026-08-30T00:00:00Z"
        }),
        pull_json(),
        pull_json(),
        json!([{
            "id": 301, "user": { "login": "reviewer" }, "body": "Guard this branch",
            "path": "src/sync.ts", "line": 12, "html_url": "https://github.test/comment/301",
            "created_at": "2026-08-30T00:10:00Z"
        }]),
        json!({ "check_runs": [{
            "id": 401, "name": "test", "status": "completed", "conclusion": "success",
            "html_url": "https://github.test/check/401", "started_at": "2026-08-30T00:11:00Z",
            "completed_at": "2026-08-30T00:12:00Z"
        }] }),
    ]));
    let provider = GitHubProvider::with_transport(
        ForgeRepository {
            host: "github.test".into(),
            owner: "acme".into(),
            name: "mars".into(),
        },
        "https://api.github.test",
        transport.clone(),
    );

    assert_eq!(
        provider
            .current_user()
            .expect("user should normalize")
            .login,
        "faye"
    );
    let issue = provider.get_issue(7).expect("issue should normalize");
    assert_eq!(issue.title, "Fix sync");
    assert_eq!(issue.labels, vec!["bug"]);
    let created = provider
        .create_pull_request(&CreatePullRequest {
            title: "Fix sync".into(),
            body: "Closes #7".into(),
            head: "task/sync".into(),
            base: "main".into(),
            draft: false,
        })
        .expect("PR should create");
    assert_eq!(created.number, 9);
    assert_eq!(
        provider.get_pull_request(9).expect("PR should load"),
        created
    );
    let comments = provider
        .list_review_comments(9)
        .expect("comments should normalize");
    assert_eq!(comments[0].path.as_deref(), Some("src/sync.ts"));
    let checks = provider
        .list_checks("abc123")
        .expect("checks should normalize");
    assert_eq!(checks[0].conclusion.as_deref(), Some("success"));

    let captured = transport.requests.lock().expect("requests should unlock");
    assert_eq!(captured.len(), 6);
    assert_eq!(captured[0].0, Method::GET);
    assert!(captured[0].1.ends_with("/user"));
    assert!(captured[1].1.ends_with("/repos/acme/mars/issues/7"));
    assert_eq!(captured[2].0, Method::POST);
    assert_eq!(
        captured[2]
            .2
            .as_ref()
            .and_then(|body| body["head"].as_str()),
        Some("task/sync")
    );
    assert!(
        captured[5]
            .1
            .ends_with("/commits/abc123/check-runs?per_page=100&page=1")
    );
}

fn pull_json() -> Value {
    json!({
        "id": 201, "number": 9, "title": "Fix sync", "body": "Closes #7", "state": "open",
        "html_url": "https://github.test/acme/mars/pull/9",
        "head": { "ref": "task/sync", "sha": "abc123" },
        "base": { "ref": "main", "sha": "base123" },
        "draft": false, "mergeable": true, "updated_at": "2026-08-30T00:05:00Z"
    })
}
