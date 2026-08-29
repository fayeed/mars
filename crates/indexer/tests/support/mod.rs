use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_security::TrustDecision;
use mars_storage::Storage;

pub struct IndexFixture {
    _directory: TestDirectory,
    #[allow(dead_code)]
    pub root: PathBuf,
    pub repository_id: String,
    pub storage: Arc<Storage>,
}

pub fn index_fixture(name: &str) -> IndexFixture {
    let directory = TestDirectory::new(name);
    let root = directory.path().join("repo");
    copy_directory(&fixture_source(), &root);
    fs::create_dir(root.join(".git")).expect("Git directory should exist");
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
        _directory: directory,
        root,
        repository_id: project.repository_id,
        storage,
    }
}

fn fixture_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/repositories/typescript-graph")
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
