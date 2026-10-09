//! Adapter streams, launch commands, and the run pump, on every platform.
//!
//! The older contract and process suites are Unix-only because they drive
//! `/bin/sh`. These use `mars-agent-fixture`, a real process built with the
//! crate, so the same assertions hold on Windows.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mars_agents::adapter::{
    AdapterKind, AgentAdapter, AgentLaunchRequest, JsonLineDecoder, McpServer,
    NormalizedAgentEvent, TOOL_OUTPUT_LIMIT, bounded_output, is_test_command,
};
use mars_agents::claude::ClaudeAdapter;
use mars_agents::codex::CodexAdapter;
use mars_agents::executables::{Environment, ExecutableSource, resolve_in};
use mars_agents::process::{ProcessSpec, ProcessSupervisor, Utf8Chunks};
use mars_agents::pump::{RunObserver, pump};
use mars_storage::Storage;
use serde_json::Value;

fn decode(adapter: &dyn AgentAdapter, stream: &str) -> Vec<NormalizedAgentEvent> {
    let mut decoder = JsonLineDecoder::new();
    let mut events = decoder.push(stream.as_bytes(), adapter);
    events.extend(decoder.finish(adapter));
    events
}

#[test]
fn claude_tool_results_pair_with_their_calls_and_reasoning_is_never_kept() {
    let stream = r#"{"type":"system","subtype":"init","session_id":"s-1","tools":["Bash"]}
{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"secret plan"},{"type":"text","text":"Running the tests."},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"pnpm --filter app test"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"1 failed"}],"is_error":true}]}}
{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}
{"type":"system","subtype":"status","status":"compacting"}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_2","name":"Edit","input":{"file_path":"src/app.ts"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_2","content":"ok"}]}}
{"type":"result","subtype":"success","is_error":false,"result":"Done","total_cost_usd":0.42,"num_turns":3,"usage":{"input_tokens":1200,"output_tokens":300,"cache_read_input_tokens":900}}
"#;
    let events = decode(&ClaudeAdapter, stream);

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, NormalizedAgentEvent::Unknown { .. })),
        "benign frames must not read as unrecognised: {events:?}"
    );
    let serialized = serde_json::to_string(&events).expect("events serialize");
    assert!(!serialized.contains("secret plan"));

    assert!(
        matches!(&events[0], NormalizedAgentEvent::Checkpoint { session_id } if session_id == "s-1")
    );
    assert!(
        matches!(&events[1], NormalizedAgentEvent::Text { text } if text == "Running the tests.")
    );
    assert!(matches!(
        &events[2],
        NormalizedAgentEvent::Test { command, call_id: Some(id), .. }
            if command == "pnpm --filter app test" && id == "toolu_1"
    ));
    assert!(matches!(
        &events[3],
        NormalizedAgentEvent::ToolResult { call_id, output, is_error: true, .. }
            if call_id == "toolu_1" && output == "1 failed"
    ));
    assert!(matches!(
        &events[4],
        NormalizedAgentEvent::FileChange { path, call_id: Some(id), .. }
            if path == "src/app.ts" && id == "toolu_2"
    ));
    assert!(matches!(
        &events[5],
        NormalizedAgentEvent::ToolResult { call_id, is_error: false, .. } if call_id == "toolu_2"
    ));
    let NormalizedAgentEvent::End {
        success: true,
        usage: Some(usage),
        ..
    } = &events[6]
    else {
        panic!(
            "the result should end the run with its usage: {:?}",
            events[6]
        );
    };
    assert_eq!(usage.cost_usd, Some(0.42));
    assert_eq!(usage.input_tokens, Some(1200));
    assert_eq!(usage.turns, Some(3));
    assert_eq!(events.len(), 7);
}

#[test]
fn codex_commands_complete_with_output_and_reasoning_is_never_kept() {
    let stream = r#"{"type":"thread.started","thread_id":"t-1"}
{"type":"turn.started"}
{"type":"item.started","item":{"id":"item_0","type":"command_execution","command":"bash -lc 'cargo test -p app'","status":"in_progress"}}
{"type":"item.completed","item":{"id":"item_0","type":"command_execution","command":"bash -lc 'cargo test -p app'","aggregated_output":"test result: FAILED","exit_code":101,"status":"failed"}}
{"type":"item.completed","item":{"id":"item_1","type":"reasoning","text":"private musing"}}
{"type":"item.started","item":{"id":"item_2","type":"mcp_tool_call","server":"mars","tool":"search","arguments":{"query":"retry"},"status":"in_progress"}}
{"type":"item.completed","item":{"id":"item_2","type":"mcp_tool_call","server":"mars","tool":"search","status":"completed","result":{"content":[{"type":"text","text":"3 hits"}]}}}
{"type":"item.started","item":{"id":"item_3","type":"file_change","changes":[{"path":"src/lib.rs","kind":"update"}],"status":"in_progress"}}
{"type":"item.completed","item":{"id":"item_3","type":"file_change","changes":[{"path":"src/lib.rs","kind":"update"}],"status":"completed"}}
{"type":"item.completed","item":{"id":"item_4","type":"agent_message","text":"Fixed."}}
{"type":"turn.completed","usage":{"input_tokens":500,"cached_input_tokens":100,"output_tokens":50}}
"#;
    let events = decode(&CodexAdapter, stream);
    let serialized = serde_json::to_string(&events).expect("events serialize");
    assert!(!serialized.contains("private musing"));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, NormalizedAgentEvent::Unknown { .. })),
        "{events:?}"
    );

    assert!(
        matches!(&events[0], NormalizedAgentEvent::Checkpoint { session_id } if session_id == "t-1")
    );
    assert!(matches!(
        &events[1],
        NormalizedAgentEvent::Test { call_id: Some(id), status, .. } if id == "item_0" && status == "started"
    ));
    assert!(matches!(
        &events[2],
        NormalizedAgentEvent::ToolResult { call_id, exit_code: Some(101), is_error: true, output }
            if call_id == "item_0" && output == "test result: FAILED"
    ));
    assert!(matches!(
        &events[3],
        NormalizedAgentEvent::Tool { name, call_id: Some(id), .. } if name == "mcp__mars__search" && id == "item_2"
    ));
    assert!(matches!(
        &events[4],
        NormalizedAgentEvent::ToolResult { call_id, output, is_error: false, .. } if call_id == "item_2" && output == "3 hits"
    ));
    // A file change is reported once, when it is complete.
    let changes = events
        .iter()
        .filter(|event| matches!(event, NormalizedAgentEvent::FileChange { .. }))
        .count();
    assert_eq!(changes, 1);
    assert!(
        matches!(events.last(), Some(NormalizedAgentEvent::End { success: true, usage: Some(usage), .. }) if usage.output_tokens == Some(50))
    );
}

fn request() -> AgentLaunchRequest {
    let mut request = AgentLaunchRequest::new(
        r"C:\Program Files\agent.exe",
        r"C:\Users\me\worktrees\task one",
        "A prompt far longer than any argument should carry",
    );
    request.allowed_tools = vec!["Read".into(), "Edit".into()];
    request.allow_write = true;
    request.mcp_servers = vec![McpServer {
        name: "mars".into(),
        command: PathBuf::from(r"C:\Program Files\Mars\mars-desktop.exe"),
        arguments: vec![
            "mcp".into(),
            "--database".into(),
            r"C:\Users\me\AppData\Roaming\dev.mars.desktop\mars.sqlite3".into(),
        ],
        environment: vec![("MARS_RUN".into(), "run-1".into())],
    }];
    request
}

fn strings(arguments: &[std::ffi::OsString]) -> Vec<String> {
    arguments
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn claude_gets_its_prompt_on_stdin_and_mars_tools_through_mcp() {
    let mut request = request();
    request.permission_prompt_tool = Some("mcp__mars__permission_prompt".into());
    request.plugin_dirs = vec![PathBuf::from(r"C:\data\runs\run-1\plugin")];
    let command = ClaudeAdapter.command(&request).expect("command builds");
    let arguments = strings(&command.arguments);

    assert_eq!(command.stdin.as_deref(), Some(request.prompt.as_str()));
    assert!(
        !arguments
            .iter()
            .any(|argument| argument.contains("far longer"))
    );
    assert!(!arguments.contains(&"--".to_owned()));
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "default"])
    );
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["--permission-prompt-tool", "mcp__mars__permission_prompt"])
    );
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["--allowedTools", "Read,Edit,mcp__mars"])
    );
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair[0] == "--plugin-dir" && pair[1].ends_with("plugin"))
    );
    assert!(
        !arguments
            .iter()
            .any(|argument| argument.contains("dangerously"))
    );

    let config_index = arguments
        .iter()
        .position(|argument| argument == "--mcp-config")
        .expect("MCP config is passed");
    let config: Value =
        serde_json::from_str(&arguments[config_index + 1]).expect("MCP config is JSON");
    assert_eq!(
        config
            .pointer("/mcpServers/mars/command")
            .and_then(Value::as_str),
        Some(r"C:\Program Files\Mars\mars-desktop.exe")
    );
    assert_eq!(
        config
            .pointer("/mcpServers/mars/env/MARS_RUN")
            .and_then(Value::as_str),
        Some("run-1")
    );
    assert!(
        command
            .environment
            .iter()
            .any(|(name, _)| name == "MCP_TOOL_TIMEOUT"),
        "a person deciding must not time the request out"
    );

    // Without anyone to ask, nothing outside the allow-list is permitted.
    let mut unattended = self::request();
    unattended.permission_prompt_tool = None;
    let arguments = strings(&ClaudeAdapter.command(&unattended).unwrap().arguments);
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "dontAsk"])
    );
}

#[test]
fn codex_keeps_its_sandbox_on_resume_and_reads_the_prompt_from_stdin() {
    let fresh = CodexAdapter.command(&request()).expect("command builds");
    let arguments = strings(&fresh.arguments);
    assert_eq!(arguments.first().map(String::as_str), Some("exec"));
    assert_eq!(arguments.last().map(String::as_str), Some("-"));
    assert_eq!(fresh.stdin.as_deref(), Some(request().prompt.as_str()));
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["-c", "sandbox_mode=\"workspace-write\""])
    );
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["-c", "approval_policy=\"never\""])
    );
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["-c", "sandbox_workspace_write.network_access=false"])
    );
    // Windows paths survive TOML: every backslash escaped.
    assert!(arguments.contains(
        &r#"mcp_servers.mars.command="C:\\Program Files\\Mars\\mars-desktop.exe""#.to_owned()
    ));
    assert!(arguments.iter().any(|argument| {
        argument.starts_with("mcp_servers.mars.args=[\"mcp\", \"--database\", \"C:\\\\Users")
    }));
    assert!(arguments.contains(&r#"mcp_servers.mars.env={ "MARS_RUN" = "run-1" }"#.to_owned()));

    let mut follow_up = request();
    follow_up.resume_session_id = Some("thread-9".into());
    let arguments = strings(&CodexAdapter.command(&follow_up).unwrap().arguments);
    assert_eq!(&arguments[..2], ["exec", "resume"]);
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["-c", "sandbox_mode=\"workspace-write\""])
    );
    assert!(!arguments.contains(&"--sandbox".to_owned()));
    assert!(!arguments.contains(&"--cd".to_owned()));
    assert_eq!(&arguments[arguments.len() - 2..], ["thread-9", "-"]);
}

#[test]
fn test_runs_are_recognised_by_their_words_not_their_letters() {
    for command in [
        "cargo test -p mars-agents",
        "pnpm --filter app test",
        "npm run test",
        "bash -lc 'go test ./...'",
        "npx vitest run",
        "python -m pytest -q",
        "C:\\tools\\jest.cmd --ci",
        "cargo build && cargo test",
    ] {
        assert!(is_test_command(command), "{command} runs tests");
    }
    for command in [
        "cat test.txt",
        "git commit -m 'fix test'",
        "ls tests/",
        "npm install",
        "rg latest",
    ] {
        assert!(!is_test_command(command), "{command} does not run tests");
    }
}

#[test]
fn bounded_output_keeps_the_head_and_the_tail() {
    let short = "all good";
    assert_eq!(bounded_output(short), short);

    let long = format!(
        "{}{}{}",
        "head ",
        "x".repeat(TOOL_OUTPUT_LIMIT * 2),
        " 3 failed ✗"
    );
    let bounded = bounded_output(&long);
    assert!(bounded.len() < TOOL_OUTPUT_LIMIT + 64);
    assert!(bounded.starts_with("head "));
    assert!(bounded.ends_with(" 3 failed ✗"));
    assert!(bounded.contains("bytes not kept"));
}

#[test]
fn utf8_chunks_hold_a_character_cut_by_a_read() {
    let text = "héllo ✓ 日本";
    let bytes = text.as_bytes();
    let mut decoder = Utf8Chunks::default();
    let mut decoded = String::new();
    for chunk in bytes.chunks(1) {
        decoded.push_str(&decoder.push(chunk));
    }
    decoded.push_str(&decoder.finish());
    assert_eq!(decoded, text);

    let mut invalid = Utf8Chunks::default();
    assert_eq!(invalid.push(b"a\xffb"), "a\u{fffd}b");
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mars-streams-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        for _ in 0..50 {
            if fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().expect("parent")).expect("parent dir");
    fs::write(path, b"").expect("file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
}

#[test]
fn executables_are_found_through_pathext_installers_and_editor_extensions() {
    let root = TempDir::new("executables");
    let bin = root.path().join("bin");
    let home = root.path().join("home");
    let app_data = root.path().join("appdata");

    // An npm install on Windows is only a `.cmd` shim.
    touch(&bin.join("claude.cmd"));
    let windows = Environment {
        path: Some(std::env::join_paths([&bin]).expect("path")),
        path_ext: Some(".COM;.EXE;.BAT;.CMD".into()),
        home: Some(home.clone()),
        app_data: Some(app_data.clone()),
        windows: true,
    };
    let found = resolve_in(AdapterKind::Claude, None, &windows).expect("shim is found");
    assert_eq!(found.path, bin.join("claude.cmd"));
    assert_eq!(found.source, ExecutableSource::Path);

    // A real executable in the same directory wins over the shim.
    touch(&bin.join("claude.exe"));
    let found = resolve_in(AdapterKind::Claude, None, &windows).expect("exe is found");
    assert_eq!(found.path, bin.join("claude.exe"));

    // A path chosen in Settings wins over everything.
    let chosen = root.path().join("chosen").join("claude.exe");
    touch(&chosen);
    let found = resolve_in(AdapterKind::Claude, Some(&chosen), &windows).expect("configured");
    assert_eq!(found.source, ExecutableSource::Configured);

    // Not on PATH: the editor extension's copy.
    let extension = home
        .join(".vscode")
        .join("extensions")
        .join("openai.chatgpt-26.1-win32-x64")
        .join("bin")
        .join("windows-x86_64")
        .join("codex.exe");
    touch(&extension);
    let found = resolve_in(AdapterKind::Codex, None, &windows).expect("extension copy");
    assert_eq!(found.path, extension);
    assert_eq!(found.source, ExecutableSource::EditorExtension);

    // The npm folder in AppData is an installer location.
    touch(&app_data.join("npm").join("codex.cmd"));
    let found = resolve_in(AdapterKind::Codex, None, &windows).expect("npm copy");
    assert_eq!(found.source, ExecutableSource::Installer);

    let nowhere = Environment {
        path: Some(std::env::join_paths([root.path().join("empty")]).expect("path")),
        home: Some(root.path().join("nobody")),
        windows: true,
        ..Environment::default()
    };
    let missing = resolve_in(AdapterKind::Claude, None, &nowhere).expect_err("nothing installed");
    assert!(!missing.searched.is_empty());
}

#[derive(Default)]
struct Collect {
    events: Vec<(NormalizedAgentEvent, bool)>,
}

struct Sink(Arc<std::sync::Mutex<Collect>>);

impl RunObserver for Sink {
    fn event(&mut self, event: NormalizedAgentEvent, stopping: bool) {
        self.0
            .lock()
            .expect("collector")
            .events
            .push((event, stopping));
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

struct PumpFixture {
    supervisor: ProcessSupervisor,
    _storage: Arc<Storage>,
    directory: TempDir,
}

impl PumpFixture {
    fn new(label: &str, delivery_capacity: usize) -> Self {
        let directory = TempDir::new(label);
        let storage = Arc::new(Storage::open(directory.path().join("mars.sqlite3")).expect("db"));
        let supervisor = ProcessSupervisor::configured(
            storage.clone(),
            delivery_capacity,
            Duration::from_millis(500),
            Arc::new(str::to_owned),
        );
        Self {
            supervisor,
            _storage: storage,
            directory,
        }
    }

    fn run(&self, script: &str, extra: &[&str], stdin: &str) -> Vec<(NormalizedAgentEvent, bool)> {
        let script_path = self.directory.path().join("script.jsonl");
        fs::write(&script_path, script).expect("script");
        let mut arguments = vec![script_path.to_string_lossy().into_owned()];
        arguments.extend(extra.iter().map(|argument| (*argument).to_owned()));
        let mut spec = ProcessSpec::new(
            "claude",
            env!("CARGO_BIN_EXE_mars-agent-fixture"),
            arguments,
            self.directory.path(),
        );
        spec.stdin = Some(stdin.to_owned());
        let collected = Arc::new(std::sync::Mutex::new(Collect::default()));
        let sink = Sink(collected.clone());
        let supervisor = self.supervisor.clone();
        runtime().block_on(async move {
            let launched = supervisor.launch(spec).expect("fixture launches");
            tokio::time::timeout(
                Duration::from_secs(30),
                pump(supervisor, Box::new(ClaudeAdapter), launched, sink),
            )
            .await
            .expect("the pump finishes");
        });
        let mut guard = collected.lock().expect("collector");
        std::mem::take(&mut guard.events)
    }
}

#[test]
fn the_pump_delivers_stdin_whole_characters_and_exactly_one_ending() {
    let fixture = PumpFixture::new("pump", 256);
    let prompt = "Fix the parser — ünïcödé prompt\nsecond line";
    let script = "#split\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"héllo ✓ 日本\"}]}}\n{\"type\":\"result\",\"is_error\":false,\"result\":\"done\"}\n";
    let events = fixture.run(script, &["--echo-stdin", "--exit", "3"], prompt);

    let texts: Vec<&str> = events
        .iter()
        .filter_map(|(event, _)| match event {
            NormalizedAgentEvent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, [prompt, "héllo ✓ 日本"]);
    let endings: Vec<&NormalizedAgentEvent> = events
        .iter()
        .map(|(event, _)| event)
        .filter(|event| matches!(event, NormalizedAgentEvent::End { .. }))
        .collect();
    // The provider said it finished; the non-zero exit after it is not a
    // second verdict.
    assert_eq!(endings.len(), 1);
    assert!(matches!(
        endings[0],
        NormalizedAgentEvent::End { success: true, .. }
    ));
}

#[test]
fn the_pump_ends_a_run_whose_provider_never_said_so() {
    let fixture = PumpFixture::new("pump-exit", 256);
    let events = fixture.run(
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"partial\"}]}}\n",
        &["--exit", "2"],
        "go",
    );
    let last = events.last().map(|(event, _)| event);
    assert!(matches!(
        last,
        Some(NormalizedAgentEvent::End { success: false, message: Some(message), .. })
            if message.contains("code 2")
    ));
}

#[test]
fn the_pump_recovers_what_live_delivery_dropped() {
    // A delivery channel of one forces the supervisor to drop live events;
    // every one of them must still reach the observer, in order.
    let fixture = PumpFixture::new("pump-lag", 1);
    let mut script = String::new();
    for index in 0..400 {
        script.push_str(&format!(
            "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"line {index}\"}}]}}}}\n"
        ));
    }
    script.push_str("{\"type\":\"result\",\"is_error\":false}\n");
    let events = fixture.run(&script, &[], "");
    let texts: Vec<String> = events
        .iter()
        .filter_map(|(event, _)| match event {
            NormalizedAgentEvent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    let expected: Vec<String> = (0..400).map(|index| format!("line {index}")).collect();
    assert_eq!(texts, expected);
    assert_eq!(
        events
            .iter()
            .filter(|(event, _)| matches!(event, NormalizedAgentEvent::End { .. }))
            .count(),
        1
    );
}
