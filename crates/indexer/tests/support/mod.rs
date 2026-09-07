use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_security::TrustDecision;
use mars_storage::Storage;

pub struct IndexFixture {
    // Declaration order is drop order: the database must close before the
    // directory holding it is removed.
    pub storage: Arc<Storage>,
    #[allow(dead_code)]
    pub root: PathBuf,
    pub repository_id: String,
    _directory: TestDirectory,
}

#[allow(dead_code)]
pub fn index_fixture(name: &str) -> IndexFixture {
    build_fixture(name, &fixture_source(), false)
}

/// A fixture covering every language the indexer parses, backed by a real Git
/// repository so the history layer has commits to read.
#[allow(dead_code)]
pub fn polyglot_fixture(name: &str) -> IndexFixture {
    build_fixture(name, &polyglot_source(), true)
}

fn build_fixture(name: &str, source: &Path, with_git: bool) -> IndexFixture {
    let directory = TestDirectory::new(name);
    let root = directory.path().join("repo");
    copy_directory(source, &root);
    if with_git {
        init_repository(&root);
    } else {
        fs::create_dir(root.join(".git")).expect("Git directory should exist");
    }
    let storage = Arc::new(
        Storage::open(directory.path().join("mars-test.sqlite3")).expect("storage should open"),
    );
    let outcome = ProjectService::new(storage.clone())
        .open_repository(&root, TrustDecision::Trust)
        .expect("fixture repository should open");
    let OpenRepositoryOutcome::Opened(project) = outcome else {
        panic!("trusted fixture should open")
    };
    IndexFixture {
        storage,
        root,
        repository_id: project.repository_id,
        _directory: directory,
    }
}

/// A generated repository of `count` source files, for cold-index timing.
#[allow(dead_code)]
pub fn scale_fixture(name: &str, count: usize) -> IndexFixture {
    let directory = TestDirectory::new(name);
    let root = directory.path().join("repo");
    fs::create_dir_all(root.join("src")).expect("scale fixture should be created");
    fs::create_dir(root.join(".git")).expect("Git directory should exist");
    for index in 0..count {
        // Each file carries several definitions and an import of its neighbour,
        // so the symbol, chunk, and edge layers all do real work.
        let previous = index.saturating_sub(1);
        let source = format!(
            "import {{ helper{previous} }} from \"./module_{previous:04}\";\n\n\
             /** Adds a pair of numbers for module {index}. */\n\
             export function helper{index}(left: number, right: number): number {{\n  \
             return left + right;\n}}\n\n\
             export class Widget{index} {{\n  \
             render(): number {{\n    return helper{previous}(1, 2);\n  }}\n}}\n"
        );
        fs::write(root.join(format!("src/module_{index:04}.ts")), source)
            .expect("scale fixture file should write");
    }

    let storage = Arc::new(
        Storage::open(directory.path().join("mars-test.sqlite3")).expect("storage should open"),
    );
    let outcome = ProjectService::new(storage.clone())
        .open_repository(&root, TrustDecision::Trust)
        .expect("scale fixture should open");
    let OpenRepositoryOutcome::Opened(project) = outcome else {
        panic!("trusted fixture should open")
    };
    IndexFixture {
        storage,
        root,
        repository_id: project.repository_id,
        _directory: directory,
    }
}

#[allow(dead_code)]
fn fixture_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/repositories/typescript-graph")
}

#[allow(dead_code)]
fn polyglot_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/repositories/polyglot")
}

/// Creates a real repository with one commit, isolated from the developer's Git
/// identity and global configuration.
#[allow(dead_code)]
pub fn git(root: &Path, arguments: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .env("GIT_CONFIG_GLOBAL", root.join("nonexistent-gitconfig"))
        .env("GIT_CONFIG_SYSTEM", root.join("nonexistent-gitconfig"))
        .env("GIT_AUTHOR_NAME", "Mars Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@mars.test")
        .env("GIT_COMMITTER_NAME", "Mars Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@mars.test")
        .output()
        .unwrap_or_else(|error| panic!("git {arguments:?} should run: {error}"));
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn init_repository(root: &Path) {
    git(root, &["init", "--initial-branch=main"]);
    git(root, &["config", "user.name", "Mars Fixture"]);
    git(root, &["config", "user.email", "fixture@mars.test"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "initial fixture commit"]);
}

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("fixture destination should exist");
    for entry in fs::read_dir(source).expect("fixture source should list") {
        let entry = entry.expect("fixture entry should load");
        let target = destination.join(entry.file_name());
        if entry
            .file_type()
            .expect("fixture type should load")
            .is_dir()
        {
            copy_directory(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("fixture file should copy");
        }
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mars-indexer-{name}-{}-{nonce}",
            std::process::id()
        ));
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
