#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use mars_agents::adapter::{
    AdapterKind, AgentAdapter, AgentLaunchRequest, JsonLineDecoder, NormalizedAgentEvent,
};
use mars_agents::claude::ClaudeAdapter;
use mars_agents::codex::CodexAdapter;

#[test]
fn provider_fixtures_satisfy_the_same_normalized_event_contract() {
    for (adapter, fixture) in adapters() {
        let contents = fs::read(fixture).expect("provider fixture should load");
        let mut decoder = JsonLineDecoder::new();
        let mut events = decoder.push(&contents, adapter.as_ref());
        events.extend(decoder.finish(adapter.as_ref()));

        assert!(
            events
                .iter()
                .any(|event| matches!(event, NormalizedAgentEvent::Checkpoint { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, NormalizedAgentEvent::Text { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, NormalizedAgentEvent::Tool { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, NormalizedAgentEvent::Command { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, NormalizedAgentEvent::FileChange { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, NormalizedAgentEvent::Test { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, NormalizedAgentEvent::Permission { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, NormalizedAgentEvent::Unknown { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, NormalizedAgentEvent::End { success: true, .. }))
        );
    }
}

#[test]
fn partial_and_malformed_jsonl_frames_degrade_without_losing_data() {
    for (adapter, _) in adapters() {
        let mut decoder = JsonLineDecoder::new();
        assert!(
            decoder
                .push(b"{\"type\":\"thread.", adapter.as_ref())
                .is_empty()
        );
        let partial = decoder.push(
            b"started\",\"thread_id\":\"partial\"}\nnot-json\n",
            adapter.as_ref(),
        );
        assert_eq!(partial.len(), 2);
        assert!(
            matches!(partial[1], NormalizedAgentEvent::Unknown { ref provider_type, .. } if provider_type == "malformed_json")
        );

        let trailing = decoder.push(b"{\"type\":\"trailing\"}", adapter.as_ref());
        assert!(trailing.is_empty());
        assert!(matches!(
            decoder.finish(adapter.as_ref()).as_slice(),
            [NormalizedAgentEvent::Unknown { .. }]
        ));
    }
}

#[test]
fn commands_use_streaming_modes_and_resume_without_permission_bypass() {
    let root = PathBuf::from("/tmp/mars adapter worktree");
    let request = AgentLaunchRequest {
        executable: PathBuf::from("/opt/bin/agent"),
        cwd: root.clone(),
        prompt: "Implement the task".into(),
        resume_session_id: None,
        allowed_tools: vec!["Read".into(), "Edit".into()],
    };

    let claude = ClaudeAdapter;
    let claude_command = claude
        .command(&request)
        .expect("Claude command should build");
    let claude_args = strings(&claude_command.arguments);
    assert!(
        claude_args
            .windows(2)
            .any(|pair| pair == ["--output-format", "stream-json"])
    );
    assert!(
        claude_args
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "dontAsk"])
    );
    assert!(
        !claude_args
            .iter()
            .any(|argument| argument.contains("dangerously"))
    );

    let codex = CodexAdapter;
    let codex_command = codex.command(&request).expect("Codex command should build");
    let codex_args = strings(&codex_command.arguments);
    assert_eq!(
        codex_args[..4],
        ["exec", "--json", "--color", "never"].map(str::to_owned)
    );
    assert!(
        codex_args
            .windows(2)
            .any(|pair| pair == ["--sandbox", "workspace-write"])
    );
    assert!(
        !codex_args
            .iter()
            .any(|argument| argument.contains("dangerously"))
    );

    let resumed = AgentLaunchRequest {
        resume_session_id: Some("session-123".into()),
        ..request
    };
    assert!(
        strings(
            &claude
                .command(&resumed)
                .expect("Claude resume should build")
                .arguments
        )
        .windows(2)
        .any(|pair| pair == ["--resume", "session-123"])
    );
    assert!(
        strings(
            &codex
                .command(&resumed)
                .expect("Codex resume should build")
                .arguments
        )
        .windows(2)
        .any(|pair| pair == ["--json", "session-123"])
    );
}

#[test]
fn probes_versions_and_normalizes_nonzero_process_exit() {
    let directory = TestDirectory::new();
    let executable = directory.path().join("fixture-agent");
    fs::write(&executable, "#!/bin/sh\nprintf 'fixture-agent 1.2.3\\n'\n")
        .expect("probe executable should write");
    let mut permissions = fs::metadata(&executable)
        .expect("probe metadata should load")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).expect("probe should be executable");

    for (adapter, _) in adapters() {
        let probe = adapter.probe(&executable).expect("probe should work");
        assert_eq!(probe.version, "fixture-agent 1.2.3");
        assert!(probe.supports_resume);
        assert!(matches!(
            adapter.on_exit(Some(9)),
            NormalizedAgentEvent::End { success: false, .. }
        ));
    }
}

#[test]
#[ignore = "requires authenticated installed Claude Code and explicitly permits a paid agent run"]
fn real_claude_smoke() {
    real_adapter_smoke(AdapterKind::Claude, Path::new("claude"));
}

#[test]
#[ignore = "requires authenticated installed Codex CLI and explicitly permits a paid agent run"]
fn real_codex_smoke() {
    real_adapter_smoke(AdapterKind::Codex, Path::new("codex"));
}

fn real_adapter_smoke(expected: AdapterKind, executable: &Path) {
    let directory = TestDirectory::new();
    let git = Command::new("git")
        .arg("-C")
        .arg(directory.path())
        .args(["init", "--quiet"])
        .output()
        .expect("Git should launch");
    assert!(git.status.success());
    let adapter: Box<dyn AgentAdapter> = match expected {
        AdapterKind::Claude => Box::new(ClaudeAdapter),
        AdapterKind::Codex => Box::new(CodexAdapter),
    };
    assert_eq!(
        adapter
            .probe(executable)
            .expect("installed CLI should probe")
            .kind,
        expected
    );
    let command = adapter
        .command(&AgentLaunchRequest {
            executable: executable.to_owned(),
            cwd: directory.path().to_owned(),
            prompt: "Reply with exactly MARS_SMOKE_OK. Do not use tools or modify files.".into(),
            resume_session_id: None,
            allowed_tools: Vec::new(),
        })
        .expect("smoke command should build");
    let output = Command::new(command.program)
        .args(command.arguments)
        .current_dir(command.cwd)
        .output()
        .expect("real adapter should launch");
    assert!(
        output.status.success(),
        "real adapter failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut decoder = JsonLineDecoder::new();
    let mut events = decoder.push(&output.stdout, adapter.as_ref());
    events.extend(decoder.finish(adapter.as_ref()));
    assert!(events.iter().any(
        |event| matches!(event, NormalizedAgentEvent::Text { text } if text.contains("MARS_SMOKE_OK"))
    ));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, NormalizedAgentEvent::End { success: true, .. }))
    );
}

fn adapters() -> Vec<(Box<dyn AgentAdapter>, PathBuf)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/agents");
    vec![
        (Box::new(ClaudeAdapter), root.join("claude/session.jsonl")),
        (Box::new(CodexAdapter), root.join("codex/session.jsonl")),
    ]
}

fn strings(arguments: &[std::ffi::OsString]) -> Vec<String> {
    arguments
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mars-adapter-fixture-{}-{nonce}",
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
        if self.0.exists() {
            fs::remove_dir_all(&self.0).expect("fixture should be removable");
        }
    }
}
