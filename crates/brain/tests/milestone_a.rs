#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mars_brain::BrainService;
use mars_core::files::FileService;
use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_core::terminal::TerminalService;
use mars_indexer::Indexer;
use mars_security::TrustDecision;
use mars_storage::Storage;
use tokio_util::sync::CancellationToken;

#[test]
fn real_repository_edit_terminal_index_and_brain_navigation() {
    let directory = TestDirectory::new_named("mars-milestone-a");
    let root = directory.path().join("separate-typescript-repository");
    fs::create_dir_all(root.join("src")).expect("source directory should exist");
    fs::write(root.join("package.json"), r#"{"name":"milestone-a-proof"}"#)
        .expect("package manifest should write");
    fs::write(
        root.join("src/main.ts"),
        "export function greet(name: string) {\n  return `Hello ${name}`;\n}\n\nexport function run() {\n  return greet(\"Mars\");\n}\n",
    )
    .expect("TypeScript source should write");
    git(&root, &["init", "--quiet"]);
    git(&root, &["add", "package.json", "src/main.ts"]);

    let storage = Arc::new(
        Storage::open(directory.path().join("mars.sqlite3")).expect("storage should open"),
    );
    let OpenRepositoryOutcome::Opened(project) = ProjectService::new(storage.clone())
        .open_repository(&root, TrustDecision::Trust)
        .expect("real Git repository should open")
    else {
        panic!("trusted repository should open")
    };

    let files = FileService::new(storage.clone());
    assert!(
        files
            .list_files(&project.repository_id, None, 50)
            .expect("repository should browse")
            .entries
            .iter()
            .any(|entry| entry.relative_path == "src/main.ts")
    );
    let original = files
        .read_file(&project.repository_id, "src/main.ts")
        .expect("source should open");
    let edited = original.content.replace("Hello", "Welcome");
    files
        .write_file(
            &project.repository_id,
            "src/main.ts",
            &edited,
            &original.version,
        )
        .expect("editor save should persist");

    let terminal_started = Instant::now();
    let terminals = TerminalService::new(storage.clone());
    let session = terminals
        .spawn_terminal(&project.repository_id, 100, 30, Arc::new(|_| {}))
        .expect("native PTY should start");
    terminals
        .write_terminal(
            &session.session_id,
            "git --no-pager -c color.ui=false diff -- src/main.ts; printf '__MARS_DIFF_DONE__\\n'\n",
        )
        .expect("terminal command should run");
    let terminal_output = wait_for_terminal(
        &terminals,
        &session.session_id,
        "+  return `Welcome ${name}`;",
    );
    assert!(
        terminal_output.contains("+  return `Welcome ${name}`;"),
        "terminal diff output was: {terminal_output:?}"
    );
    terminals
        .cancel_terminal(&session.session_id)
        .expect("terminal should stop");
    let terminal_elapsed = terminal_started.elapsed();

    let index_started = Instant::now();
    let summary = Indexer::new(storage.clone())
        .index_repository(&project.repository_id, &CancellationToken::new())
        .expect("edited repository should index");
    let index_elapsed = index_started.elapsed();
    assert_eq!(summary.indexed_files, 1);

    let query_started = Instant::now();
    let brain = BrainService::new(storage);
    let greet = brain
        .find_symbol(&project.repository_id, "greet", 10)
        .expect("Brain search should work")
        .into_iter()
        .find(|symbol| symbol.name == "greet")
        .expect("greet definition should be indexed");
    let neighborhood = brain
        .related_code(&project.repository_id, &greet.id, 2, 25)
        .expect("related code should load");
    let query_elapsed = query_started.elapsed();
    assert!(
        neighborhood
            .symbols
            .iter()
            .any(|symbol| symbol.name == "run")
    );

    let navigated = files
        .read_file(&project.repository_id, &greet.relative_path)
        .expect("Brain source evidence should reopen");
    assert!(
        navigated
            .content
            .lines()
            .nth(greet.start_line - 1)
            .is_some_and(|line| line.contains("function greet"))
    );

    eprintln!(
        "milestone-a timings: terminal_diff_ms={}, index_ms={}, query_ms={}",
        terminal_elapsed.as_millis(),
        index_elapsed.as_millis(),
        query_elapsed.as_millis()
    );
}

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .expect("system Git should launch");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new_named(prefix: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("test directory should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.0.exists() {
            fs::remove_dir_all(&self.0).expect("test directory should be removable");
        }
    }
}

fn wait_for_terminal(service: &TerminalService, session_id: &str, marker: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let output = service
            .terminal_output(session_id)
            .expect("terminal output should load");
        if output.contains(marker) {
            return output;
        }
        assert!(
            Instant::now() < deadline,
            "terminal did not finish the Git diff command"
        );
        thread::sleep(Duration::from_millis(20));
    }
}
