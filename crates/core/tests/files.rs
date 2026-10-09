use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mars_core::files::{FileError, FileService, MAX_EDITABLE_FILE_BYTES};
use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_security::TrustDecision;
use mars_storage::Storage;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("mars-files-{name}-{}-{nonce}", std::process::id()));
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

fn trusted_repository(fixture: &TestDirectory) -> (Arc<Storage>, PathBuf, String) {
    let repository = fixture.path().join("repo");
    fs::create_dir_all(repository.join(".git")).expect("Git directory should exist");
    let storage = Arc::new(
        Storage::open(fixture.path().join("mars-test.sqlite3")).expect("storage should open"),
    );
    let outcome = ProjectService::new(storage.clone())
        .open_repository(&repository, TrustDecision::Trust)
        .expect("repository should open");
    let OpenRepositoryOutcome::Opened(project) = outcome else {
        panic!("trusted repository should open")
    };
    (storage, repository, project.repository_id)
}

#[test]
fn listing_respects_git_product_and_default_ignores_with_pagination() {
    let fixture = TestDirectory::new("listing");
    let (storage, repository, repository_id) = trusted_repository(&fixture);
    fs::write(repository.join(".gitignore"), "ignored.ts\n").expect("ignore file should write");
    fs::write(repository.join(".marsignore"), "private.ts\n").expect("ignore file should write");
    fs::write(repository.join("a.ts"), "export const a = 1;\n").expect("file should write");
    fs::write(repository.join("b.ts"), "export const b = 2;\n").expect("file should write");
    fs::write(repository.join("ignored.ts"), "ignored\n").expect("file should write");
    fs::write(repository.join("private.ts"), "private\n").expect("file should write");
    fs::write(repository.join(".env"), "TOKEN=secret\n").expect("secret should write");
    fs::create_dir_all(repository.join("node_modules/pkg")).expect("directory should exist");
    fs::write(repository.join("node_modules/pkg/index.js"), "dependency\n")
        .expect("file should write");
    fs::write(repository.join(".git/config"), "metadata\n").expect("file should write");
    let service = FileService::new(storage);

    let first = service
        .list_files(&repository_id, None, 2)
        .expect("first page should load");
    assert_eq!(first.entries.len(), 2);
    assert!(first.next_cursor.is_some());

    let mut paths: Vec<_> = first
        .entries
        .iter()
        .map(|entry| entry.relative_path.clone())
        .collect();
    let mut cursor = first.next_cursor;
    while cursor.is_some() {
        let page = service
            .list_files(&repository_id, cursor.as_deref(), 2)
            .expect("next page should load");
        paths.extend(page.entries.into_iter().map(|entry| entry.relative_path));
        cursor = page.next_cursor;
    }

    assert!(paths.contains(&".gitignore".to_owned()));
    assert!(paths.contains(&".marsignore".to_owned()));
    assert!(paths.contains(&"a.ts".to_owned()));
    assert!(paths.contains(&"b.ts".to_owned()));
    assert!(!paths.contains(&".env".to_owned()));
    assert!(!paths.contains(&"ignored.ts".to_owned()));
    assert!(!paths.contains(&"private.ts".to_owned()));
    assert!(!paths.contains(&"node_modules/pkg/index.js".to_owned()));
    assert!(!paths.contains(&".git/config".to_owned()));
}

#[test]
fn top_directories_count_only_what_a_listing_shows_largest_first() {
    let fixture = TestDirectory::new("top-directories");
    let (storage, repository, repository_id) = trusted_repository(&fixture);
    for (path, contents) in [
        ("apps/web/a.ts", "a"),
        ("apps/web/b.ts", "b"),
        ("apps/api/c.ts", "c"),
        ("lib/d.ts", "d"),
        ("README.md", "readme"),
        ("node_modules/pkg/index.js", "dependency"),
        ("out/bundle.js", "built"),
    ] {
        let path = repository.join(path);
        fs::create_dir_all(path.parent().expect("parent")).expect("directory should exist");
        fs::write(path, contents).expect("file should write");
    }
    fs::write(repository.join(".gitignore"), "out/\n").expect("ignore file should write");

    let directories = FileService::new(storage)
        .top_directories(&repository_id)
        .expect("directories should list");
    let named: Vec<_> = directories
        .iter()
        .map(|directory| (directory.name.as_str(), directory.files))
        .collect();
    assert_eq!(named, [("apps", 3), ("lib", 1)]);
}

#[test]
fn reads_reject_binary_oversized_and_escaping_paths() {
    let fixture = TestDirectory::new("read-guards");
    let (storage, repository, repository_id) = trusted_repository(&fixture);
    fs::write(repository.join("binary.bin"), [0, 1, 2, 3]).expect("binary should write");
    fs::write(
        repository.join("large.txt"),
        vec![b'x'; MAX_EDITABLE_FILE_BYTES as usize + 1],
    )
    .expect("large file should write");
    let service = FileService::new(storage);

    assert!(matches!(
        service.read_file(&repository_id, "binary.bin"),
        Err(FileError::BinaryFile(_))
    ));
    assert!(matches!(
        service.read_file(&repository_id, "large.txt"),
        Err(FileError::FileTooLarge { .. })
    ));
    assert!(matches!(
        service.read_file(&repository_id, "../outside.txt"),
        Err(FileError::InvalidRelativePath(_))
    ));
    assert!(matches!(
        service.read_file(&repository_id, ".env"),
        Err(FileError::IgnoredPath(_))
    ));
}

#[cfg(unix)]
#[test]
fn reads_do_not_follow_symlinks_outside_the_repository() {
    use std::os::unix::fs::symlink;

    let fixture = TestDirectory::new("symlink-escape");
    let (storage, repository, repository_id) = trusted_repository(&fixture);
    let outside = fixture.path().join("outside.txt");
    fs::write(&outside, "must stay outside\n").expect("outside file should write");
    symlink(&outside, repository.join("linked.txt")).expect("symlink should be created");
    let service = FileService::new(storage);

    assert!(matches!(
        service.read_file(&repository_id, "linked.txt"),
        Err(FileError::IgnoredPath(_) | FileError::OutsideRepository(_))
    ));
}

#[test]
fn saves_are_versioned_and_refuse_external_edit_conflicts() {
    let fixture = TestDirectory::new("save-conflict");
    let (storage, repository, repository_id) = trusted_repository(&fixture);
    let source = repository.join("src.ts");
    fs::write(&source, "export const value = 1;\n").expect("source should write");
    let service = FileService::new(storage);
    let original = service
        .read_file(&repository_id, "src.ts")
        .expect("source should load");

    fs::write(&source, "export const value = 2;\n").expect("external edit should write");
    let error = service
        .write_file(
            &repository_id,
            "src.ts",
            "export const value = 3;\n",
            &original.version,
        )
        .expect_err("stale save should fail");

    assert!(matches!(error, FileError::WriteConflict { .. }));
    assert_eq!(
        fs::read_to_string(&source).expect("source should remain readable"),
        "export const value = 2;\n"
    );
}

#[test]
fn saves_replace_content_and_return_the_new_version() {
    let fixture = TestDirectory::new("save");
    let (storage, repository, repository_id) = trusted_repository(&fixture);
    fs::write(repository.join("src.ts"), "before\n").expect("source should write");
    let service = FileService::new(storage);
    let original = service
        .read_file(&repository_id, "src.ts")
        .expect("source should load");

    let saved = service
        .write_file(&repository_id, "src.ts", "after\n", &original.version)
        .expect("source should save");

    assert_ne!(saved.version, original.version);
    assert_eq!(saved.content, "after\n");
    assert_eq!(
        service
            .read_file(&repository_id, "src.ts")
            .expect("saved source should load"),
        saved
    );
}
