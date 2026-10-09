//! Reading what an agent CLI is signed in with, and what a run would bill.
//!
//! The fixtures are the CLIs' own output with account details replaced, so
//! these pin the parsers to what the tested versions actually print.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use mars_agents::account::{
    AgentAccount, Billing, SignInPreferences, present_variables, read_account,
};
use mars_agents::adapter::{AdapterKind, AgentAdapter};
use mars_agents::claude::ClaudeAdapter;
use mars_agents::codex::CodexAdapter;

#[test]
fn claude_reports_a_plan_sign_in_with_its_tier() {
    let status = fixture("claude/auth-status/plan.json");
    assert_eq!(
        ClaudeAdapter.parse_sign_in(true, &status, &[]),
        Billing::Plan {
            plan: Some("team".into())
        }
    );
}

#[test]
fn claude_signed_out_is_read_from_the_json_not_the_exit_status() {
    let status = fixture("claude/auth-status/signed-out.json");
    assert_eq!(
        ClaudeAdapter.parse_sign_in(false, &status, &[]),
        Billing::SignedOut
    );
}

#[test]
fn claude_on_a_cloud_provider_names_the_provider() {
    let status = fixture("claude/auth-status/bedrock.json");
    assert_eq!(
        ClaudeAdapter.parse_sign_in(true, &status, &[]),
        Billing::CloudProvider {
            provider: "bedrock".into()
        }
    );
}

#[test]
fn a_claude_key_source_outranks_the_sign_in_it_sits_beside() {
    let status = br#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","apiKeySource":"apiKeyHelper","subscriptionType":"max"}"#;
    assert_eq!(
        ClaudeAdapter.parse_sign_in(true, status, &[]),
        Billing::ApiKey {
            source: "apiKeyHelper".into()
        }
    );
}

#[test]
fn a_claude_without_the_status_command_is_unknown_rather_than_signed_out() {
    let billing = ClaudeAdapter.parse_sign_in(false, b"", b"error: unknown command 'auth'");
    assert!(matches!(billing, Billing::Unknown { .. }));
    assert!(!billing.is_signed_in());
}

#[test]
fn codex_reads_its_answer_from_stderr() {
    assert_eq!(
        CodexAdapter.parse_sign_in(true, &[], &fixture("codex/login-status/chatgpt.stderr")),
        Billing::Plan { plan: None }
    );
    assert_eq!(
        CodexAdapter.parse_sign_in(false, &[], &fixture("codex/login-status/signed-out.stderr")),
        Billing::SignedOut
    );
    assert_eq!(
        CodexAdapter.parse_sign_in(true, &[], b"Logged in using an API key - sk-***\n"),
        Billing::ApiKey {
            source: "saved API key".into()
        }
    );
}

#[test]
fn an_unrecognised_codex_failure_keeps_its_own_last_line() {
    let billing =
        CodexAdapter.parse_sign_in(false, &[], b"WARNING: noise\nerror: config is invalid\n");
    assert_eq!(
        billing,
        Billing::Unknown {
            detail: "error: config is invalid".into()
        }
    );
}

#[test]
fn a_variable_in_the_environment_takes_billing_from_the_plan() {
    let account = resolve_claude(plan("team"), &["ANTHROPIC_API_KEY"], false);
    assert_eq!(
        account.billing,
        Billing::ApiKey {
            source: "ANTHROPIC_API_KEY".into()
        }
    );
    assert_eq!(account.sign_in, plan("team"));
    assert!(account.removed_environment().is_empty());
}

#[test]
fn choosing_the_sign_in_bills_the_plan_and_removes_every_overriding_variable() {
    let account = resolve_claude(
        plan("team"),
        &["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"],
        true,
    );
    assert_eq!(account.billing, plan("team"));
    assert_eq!(
        account.removed_environment(),
        vec![
            OsString::from("ANTHROPIC_AUTH_TOKEN"),
            OsString::from("ANTHROPIC_API_KEY")
        ]
    );
}

#[test]
fn choosing_a_sign_in_that_does_not_exist_leaves_the_key_in_place() {
    for sign_in in [
        Billing::SignedOut,
        Billing::Unknown {
            detail: "old CLI".into(),
        },
    ] {
        let account = resolve_claude(sign_in, &["ANTHROPIC_API_KEY"], true);
        assert_eq!(
            account.billing,
            Billing::ApiKey {
                source: "ANTHROPIC_API_KEY".into()
            }
        );
        assert!(account.removed_environment().is_empty());
    }
}

#[test]
fn a_cloud_provider_is_not_overridden_by_key_variables() {
    let bedrock = Billing::CloudProvider {
        provider: "bedrock".into(),
    };
    let account = resolve_claude(bedrock.clone(), &["ANTHROPIC_API_KEY"], false);
    assert_eq!(account.billing, bedrock);
    assert!(account.overrides.is_empty());
}

#[test]
fn signed_out_with_nothing_overriding_stays_signed_out() {
    let account = resolve_claude(Billing::SignedOut, &[], false);
    assert_eq!(account.billing, Billing::SignedOut);
    assert_eq!(
        account.sign_in_command.as_deref(),
        Some("claude auth login")
    );
}

#[test]
fn only_variables_with_a_value_count_as_present() {
    let present = present_variables(ClaudeAdapter.overriding_variables(), |name| match name {
        "ANTHROPIC_AUTH_TOKEN" => Some(OsString::new()),
        "ANTHROPIC_API_KEY" => Some(OsString::from("sk-ant-test")),
        _ => None,
    });
    assert_eq!(present, vec!["ANTHROPIC_API_KEY".to_owned()]);
}

#[test]
fn the_sign_in_preference_survives_a_restart_and_can_be_withdrawn() {
    let directory = TestDirectory::new();
    let path = directory.path().join("nested").join("agent-sign-in.json");

    let preferences = SignInPreferences::load(&path);
    assert!(!preferences.uses_sign_in(AdapterKind::Claude));
    preferences
        .set(AdapterKind::Claude, true)
        .expect("preference should save");

    let reloaded = SignInPreferences::load(&path);
    assert!(reloaded.uses_sign_in(AdapterKind::Claude));
    assert!(!reloaded.uses_sign_in(AdapterKind::Codex));

    reloaded
        .set(AdapterKind::Claude, false)
        .expect("preference should save");
    assert!(!SignInPreferences::load(&path).uses_sign_in(AdapterKind::Claude));
}

#[test]
fn a_damaged_preference_file_means_nothing_was_chosen() {
    let directory = TestDirectory::new();
    let path = directory.path().join("agent-sign-in.json");
    fs::write(&path, b"{not json").expect("fixture should write");
    assert!(!SignInPreferences::load(&path).uses_sign_in(AdapterKind::Claude));
}

#[test]
#[ignore = "requires an installed Claude Code; MARS_CLAUDE_EXECUTABLE names one that is not on PATH"]
fn real_claude_says_what_it_is_signed_in_with() {
    let account = read_account(
        &ClaudeAdapter,
        &installed("MARS_CLAUDE_EXECUTABLE", "claude"),
        false,
    )
    .expect("installed Claude Code should answer");
    // Printed because this is run by hand, to see what a machine is set up with.
    println!("{account:?}");
    assert!(
        !matches!(account.sign_in, Billing::Unknown { .. }),
        "{account:?}"
    );
}

#[test]
#[ignore = "requires an installed Codex CLI; MARS_CODEX_EXECUTABLE names one that is not on PATH"]
fn real_codex_says_what_it_is_signed_in_with() {
    let account = read_account(
        &CodexAdapter,
        &installed("MARS_CODEX_EXECUTABLE", "codex"),
        false,
    )
    .expect("installed Codex CLI should answer");
    println!("{account:?}");
    assert!(
        !matches!(account.sign_in, Billing::Unknown { .. }),
        "{account:?}"
    );
}

fn installed(variable: &str, default: &str) -> PathBuf {
    std::env::var_os(variable).map_or_else(|| PathBuf::from(default), PathBuf::from)
}

fn resolve_claude(sign_in: Billing, overrides: &[&str], use_sign_in: bool) -> AgentAccount {
    AgentAccount::resolve(
        AdapterKind::Claude,
        ClaudeAdapter.sign_in_command(),
        sign_in,
        overrides.iter().map(|name| (*name).to_owned()).collect(),
        use_sign_in,
    )
}

fn plan(tier: &str) -> Billing {
    Billing::Plan {
        plan: Some(tier.into()),
    }
}

fn fixture(relative: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/agents")
        .join(relative);
    fs::read(&path).unwrap_or_else(|error| panic!("{} should load: {error}", path.display()))
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mars-account-fixture-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("fixture directory should exist");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
