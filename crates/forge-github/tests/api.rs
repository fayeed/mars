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

fn provider_with(transport: Arc<FakeTransport>) -> GitHubProvider {
    GitHubProvider::with_transport(
        ForgeRepository {
            host: "github.test".into(),
            owner: "acme".into(),
            name: "mars".into(),
        },
        "https://api.github.test",
        transport,
    )
}

#[test]
fn issues_lists_leave_pull_requests_out_and_ask_for_recent_activity_first() {
    let issue = json!({
        "id": 1, "number": 7, "title": "Sync drops events", "body": null, "state": "open",
        "html_url": "https://github.test/acme/mars/issues/7", "user": { "login": "faye" },
        "labels": [{ "name": "bug" }], "updated_at": "2026-10-01T00:00:00Z"
    });
    let pull_as_issue = json!({
        "id": 2, "number": 9, "title": "Fix sync", "body": "", "state": "open",
        "html_url": "https://github.test/acme/mars/pull/9", "user": { "login": "claude" },
        "labels": [], "updated_at": "2026-10-02T00:00:00Z",
        "pull_request": { "url": "https://api.github.test/repos/acme/mars/pulls/9" }
    });
    let transport = Arc::new(FakeTransport::with_responses(vec![
        json!([issue, pull_as_issue]),
        json!([pull_json()]),
    ]));
    let provider = provider_with(transport.clone());

    let issues = provider
        .list_issues(mars_forge::ItemState::Open, 1)
        .expect("issues list");
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].number, 7);
    assert_eq!(issues[0].labels, vec!["bug".to_owned()]);

    let pulls = provider
        .list_pull_requests(mars_forge::ItemState::Open, 2)
        .expect("pulls list");
    assert_eq!(pulls[0].number, 9);

    let requests = transport.requests.lock().expect("requests");
    assert!(requests[0].1.ends_with(
        "/repos/acme/mars/issues?state=open&sort=updated&direction=desc&per_page=100&page=1"
    ));
    assert!(
        requests[1]
            .1
            .contains("/pulls?state=open&sort=updated&direction=desc&per_page=50&page=2")
    );
}

#[test]
fn comments_are_read_and_posted_on_the_conversation() {
    let comment = json!({
        "id": 30, "user": { "login": "faye" }, "body": "Looks good",
        "html_url": "https://github.test/acme/mars/pull/9#issuecomment-30",
        "created_at": "2026-10-03T00:00:00Z"
    });
    let transport = Arc::new(FakeTransport::with_responses(vec![
        json!([comment.clone()]),
        comment,
    ]));
    let provider = provider_with(transport.clone());
    let comments = provider.list_comments(9).expect("comments");
    assert_eq!(comments[0].body, "Looks good");
    let posted = provider.comment(9, "Looks good").expect("posted");
    assert_eq!(posted.author, "faye");
    assert!(matches!(
        provider.comment(9, "   "),
        Err(ForgeError::Request(_))
    ));

    let requests = transport.requests.lock().expect("requests");
    assert_eq!(requests[1].0, Method::POST);
    assert!(requests[1].1.ends_with("/issues/9/comments"));
    assert_eq!(requests[1].2, Some(json!({ "body": "Looks good" })));
}

#[test]
fn a_merge_is_pinned_to_the_head_that_was_reviewed() {
    let transport = Arc::new(FakeTransport::with_responses(vec![json!({
        "sha": "merged123", "merged": true, "message": "Pull Request successfully merged"
    })]));
    let provider = provider_with(transport.clone());
    let outcome = provider
        .merge_pull_request(9, mars_forge::MergeMethod::Squash, "abc123")
        .expect("merge");
    assert!(outcome.merged);
    assert_eq!(outcome.sha.as_deref(), Some("merged123"));
    let requests = transport.requests.lock().expect("requests");
    assert_eq!(requests[0].0, Method::PUT);
    assert!(requests[0].1.ends_with("/pulls/9/merge"));
    assert_eq!(
        requests[0].2,
        Some(json!({ "merge_method": "squash", "sha": "abc123" }))
    );
}

#[test]
fn an_issue_list_reads_past_pages_of_pull_requests() {
    let pull_as_issue = |number: u64| {
        json!({
            "id": number, "number": number, "title": "A pull request", "body": "",
            "state": "open", "html_url": format!("https://github.test/acme/mars/pull/{number}"),
            "user": { "login": "bot" }, "labels": [], "updated_at": "2026-10-02T00:00:00Z",
            "pull_request": { "url": "https://api.github.test/repos/acme/mars/pulls/1" }
        })
    };
    let first_page: Vec<_> = (1..=100).map(pull_as_issue).collect();
    let issue = json!({
        "id": 500, "number": 500, "title": "A real issue", "body": null, "state": "open",
        "html_url": "https://github.test/acme/mars/issues/500", "user": { "login": "faye" },
        "labels": [], "updated_at": "2026-09-01T00:00:00Z"
    });
    let transport = Arc::new(FakeTransport::with_responses(vec![
        json!(first_page),
        json!([issue]),
    ]));
    let provider = provider_with(transport.clone());

    let issues = provider
        .list_issues(mars_forge::ItemState::Open, 1)
        .expect("issues list");
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].number, 500);
    let requests = transport.requests.lock().expect("requests");
    assert_eq!(requests.len(), 2);
    assert!(requests[1].1.ends_with("per_page=100&page=2"));
}
