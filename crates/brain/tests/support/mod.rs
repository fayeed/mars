use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_indexer::Indexer;
use mars_indexer::api::IndexApi;
use mars_security::TrustDecision;
use mars_storage::Storage;
use tokio_util::sync::CancellationToken;

pub struct BrainFixture {
    // Declaration order is drop order: the database must close before the
    // directory holding it is removed.
    pub storage: Arc<Storage>,
    #[allow(dead_code)]
    pub root: PathBuf,
    pub repository_id: String,
    _directory: TestDirectory,
}

pub fn indexed_fixture() -> BrainFixture {
    let directory = TestDirectory::new();
    let root = directory.path().join("repo");
    copy_directory(&fixture_source(), &root);
    fs::create_dir(root.join(".git")).expect("Git directory should exist");
    let storage = Arc::new(
        Storage::open(directory.path().join("mars-test.sqlite3")).expect("storage should open"),
    );
    let outcome = ProjectService::new(storage.clone())
        .open_repository(&root, TrustDecision::Trust)
        .expect("repository should open");
    let OpenRepositoryOutcome::Opened(project) = outcome else {
        panic!("trusted repository should open")
    };
    Indexer::new(storage.clone())
        .index_repository(&project.repository_id, &CancellationToken::new())
        .expect("fixture should index");
    BrainFixture {
        storage,
        root,
        repository_id: project.repository_id,
        _directory: directory,
    }
}

/// The ten-language fixture, backed by a real Git repository so the history
/// layer has commits to read.
#[allow(dead_code)]
pub fn polyglot_fixture() -> BrainFixture {
    let directory = TestDirectory::new();
    let root = directory.path().join("repo");
    copy_directory(&polyglot_source(), &root);
    init_repository(&root);
    let storage = Arc::new(
        Storage::open(directory.path().join("mars-test.sqlite3")).expect("storage should open"),
    );
    let outcome = ProjectService::new(storage.clone())
        .open_repository(&root, TrustDecision::Trust)
        .expect("repository should open");
    let OpenRepositoryOutcome::Opened(project) = outcome else {
        panic!("trusted repository should open")
    };
    Indexer::new(storage.clone())
        .index_repository(&project.repository_id, &CancellationToken::new())
        .expect("fixture should index");
    BrainFixture {
        storage,
        root,
        repository_id: project.repository_id,
        _directory: directory,
    }
}

impl BrainFixture {
    /// A Project Brain over this fixture, using the deterministic synthesizer.
    #[allow(dead_code)]
    pub fn brain(&self) -> mars_brain::ProjectBrain {
        mars_brain::ProjectBrain::new(self.storage.clone(), self.index())
    }

    /// A Project Brain over this fixture with a supplied synthesis provider.
    #[allow(dead_code)]
    pub fn brain_with(
        &self,
        provider: Arc<dyn mars_brain::SynthesisProvider>,
    ) -> mars_brain::ProjectBrain {
        mars_brain::ProjectBrain::with_provider(self.storage.clone(), self.index(), provider)
    }

    #[allow(dead_code)]
    pub fn index(&self) -> Arc<IndexApi> {
        Arc::new(IndexApi::new(self.storage.clone()))
    }

    /// Reads a file from the fixture working tree.
    #[allow(dead_code)]
    pub fn read(&self, relative_path: &str) -> String {
        fs::read_to_string(self.root.join(relative_path))
            .unwrap_or_else(|error| panic!("{relative_path} should be readable: {error}"))
    }

    /// Writes a file in the fixture working tree without re-indexing, which is
    /// how a test produces evidence that has drifted from the index.
    #[allow(dead_code)]
    pub fn write(&self, relative_path: &str, content: &str) {
        fs::write(self.root.join(relative_path), content)
            .unwrap_or_else(|error| panic!("{relative_path} should be writable: {error}"));
    }

    /// The id of the first symbol with this name.
    #[allow(dead_code)]
    pub fn symbol_id(&self, name: &str) -> String {
        self.storage
            .with_connection(|connection| {
                connection.query_row(
                    "SELECT s.id FROM symbols s JOIN files f ON f.id = s.file_id
                     WHERE f.repository_id = ?1 AND s.name = ?2 AND f.deleted_at IS NULL
                     ORDER BY f.relative_path, s.start_line LIMIT 1",
                    rusqlite::params![self.repository_id, name],
                    |row| row.get::<_, String>(0),
                )
            })
            .unwrap_or_else(|error| panic!("symbol `{name}` should be indexed: {error}"))
    }
}

fn init_repository(root: &Path) {
    let git = |arguments: &[&str]| {
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
    };
    git(&["init", "--initial-branch=main"]);
    git(&["config", "user.name", "Mars Fixture"]);
    git(&["config", "user.email", "fixture@mars.test"]);
    git(&["add", "-A"]);
    git(&["commit", "-m", "initial fixture commit"]);
}

fn fixture_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/repositories/typescript-graph")
}

#[allow(dead_code)]
fn polyglot_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/repositories/polyglot")
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
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mars-brain-{}-{nonce}", std::process::id()));
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
