use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_git::TaskWorktreeService;
use mars_security::TrustDecision;
use mars_skills::{
    AdapterPolicy, LocalOrigin, RegistryClient, SkillCandidate, SkillError, SkillFile, SkillScope,
    SkillService, SkillSnapshot, SkillsShRegistry, discover_local, front_matter_field,
    registry_digest,
};
use mars_storage::Storage;

#[test]
fn inspection_is_mandatory_and_installation_is_pinned_and_adapter_scoped() {
    let fixture = Fixture::new(valid_files());

    assert!(matches!(
        fixture.service.trust_and_install(
            "not-inspected",
            Some(&fixture.repository_id),
            Some(&fixture.task_id),
            SkillScope::Task,
            true,
            false,
        ),
        Err(SkillError::NotInspected(_))
    ));
    let inspected = fixture
        .service
        .inspect(&candidate())
        .expect("skill should inspect");
    assert!(inspected.skill_markdown.contains("# Demo skill"));
    assert_eq!(inspected.trust_state, "inspected");
    assert!(!fixture.install_root.exists());

    let installed = fixture
        .service
        .trust_and_install(
            &inspected.id,
            Some(&fixture.repository_id),
            Some(&fixture.task_id),
            SkillScope::Task,
            true,
            false,
        )
        .expect("inspected skill should install after trust");
    assert_eq!(installed.pinned_hash, inspected.content_hash);
    assert!(installed.install_path.join("SKILL.md").is_file());
    assert!(installed.install_path.join("scripts/check.sh").is_file());
    assert!(installed.allow_claude);
    assert!(!installed.allow_codex);
    let policies: Vec<(String, bool)> = fixture
        .storage
        .with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT adapter_kind, allowed FROM skill_agent_policies
                 WHERE installation_id = ?1 ORDER BY adapter_kind",
            )?;
            statement
                .query_map([installed.id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .expect("policies should load");
    assert_eq!(
        policies,
        vec![("claude".into(), true), ("codex".into(), false)]
    );

    fixture
        .service
        .block(&inspected.id)
        .expect("skill should block");
    assert!(matches!(
        fixture.service.trust_and_install(
            &inspected.id,
            Some(&fixture.repository_id),
            Some(&fixture.task_id),
            SkillScope::Task,
            true,
            true,
        ),
        Err(SkillError::Blocked(_))
    ));
}

#[test]
fn changed_snapshots_require_reinspection_and_report_the_diff() {
    let fixture = Fixture::new(valid_files());
    let first = fixture
        .service
        .inspect(&candidate())
        .expect("first snapshot should inspect");
    fixture.registry.set_files(vec![
        SkillFile {
            path: "SKILL.md".into(),
            contents: "---\nname: demo\ndescription: changed\n---\n# Demo skill\nChanged.\n".into(),
        },
        SkillFile {
            path: "references/new.md".into(),
            contents: "new reference".into(),
        },
    ]);

    let update = fixture
        .service
        .inspect(&candidate())
        .expect("update should inspect");

    assert_eq!(update.id, first.id);
    assert_eq!(update.trust_state, "inspected");
    assert_eq!(
        update.changed_files,
        vec!["SKILL.md", "references/new.md", "scripts/check.sh"],
    );
    assert_ne!(update.content_hash, first.content_hash);
}

#[test]
fn missing_manifest_unsafe_paths_and_hash_mismatch_are_quarantined() {
    let missing = Fixture::new(vec![SkillFile {
        path: "README.md".into(),
        contents: "not a skill".into(),
    }]);
    assert!(matches!(
        missing.service.inspect(&candidate()),
        Err(SkillError::MissingSkillMarkdown)
    ));

    let unsafe_fixture = Fixture::new(vec![SkillFile {
        path: "../SKILL.md".into(),
        contents: "unsafe".into(),
    }]);
    assert!(matches!(
        unsafe_fixture.service.inspect(&candidate()),
        Err(SkillError::UnsafePath(_))
    ));

    let mismatch = Fixture::new_with_hash(valid_files(), "incorrect");
    assert!(matches!(
        mismatch.service.inspect(&candidate()),
        Err(SkillError::InvalidRegistryHash(_))
    ));
}

#[test]
fn failed_install_cleans_staging_and_does_not_activate() {
    let fixture = Fixture::new(valid_files());
    let inspected = fixture
        .service
        .inspect(&candidate())
        .expect("skill should inspect");
    fs::write(&fixture.install_root, "blocks directory creation")
        .expect("install root blocker should write");

    assert!(matches!(
        fixture.service.trust_and_install(
            &inspected.id,
            Some(&fixture.repository_id),
            Some(&fixture.task_id),
            SkillScope::Task,
            true,
            true,
        ),
        Err(SkillError::Filesystem { .. })
    ));
    let (trust, installations): (String, i64) = fixture
        .storage
        .with_connection(|connection| {
            Ok((
                connection.query_row(
                    "SELECT trust_state FROM skills WHERE id = ?1",
                    [&inspected.id],
                    |row| row.get(0),
                )?,
                connection.query_row("SELECT COUNT(*) FROM skill_installations", [], |row| {
                    row.get(0)
                })?,
            ))
        })
        .expect("skill state should load");
    assert_eq!(trust, "inspected");
    assert_eq!(installations, 0);
}

#[test]
fn the_registry_digest_matches_the_scheme_skills_sh_publishes() {
    // Case-insensitive path order, each file as path NUL contents NUL. The
    // expected value is computed independently of this crate.
    let files = vec![
        SkillFile {
            path: "SKILL.md".into(),
            contents: "---\nname: demo\n---\n".into(),
        },
        SkillFile {
            path: "scripts/run.sh".into(),
            contents: "echo hi\n".into(),
        },
    ];
    assert_eq!(
        registry_digest(&files),
        "dc5a2aec579ec16f8c68ada0f2b5e855e3c193cc1edafa4a925c8512055ecb0e"
    );

    let fixture = Fixture::new(valid_files());
    let inspected = fixture.service.inspect(&candidate()).expect("inspects");
    assert!(inspected.registry_verified);
    let tampered = Fixture::new_with_hash(valid_files(), &"0".repeat(64));
    let inspected = tampered.service.inspect(&candidate()).expect("inspects");
    assert!(
        !inspected.registry_verified,
        "a digest that does not match is said, not hidden"
    );
}

#[test]
fn an_installation_keeps_its_pinned_version_while_a_newer_one_is_inspected() {
    let fixture = Fixture::new(valid_files());
    let first = fixture.service.inspect(&candidate()).expect("inspects");
    let pinned = fixture
        .service
        .trust_and_install(
            &first.id,
            Some(&fixture.repository_id),
            Some(&fixture.task_id),
            SkillScope::Task,
            true,
            true,
        )
        .expect("installs");
    assert_eq!(pinned.description, "Test skill");

    let mut changed = valid_files();
    changed[0].contents = "---\nname: demo\ndescription: Changed\n---\n# Changed\n".into();
    fixture.registry.set_files(changed);
    let update = fixture.service.check_update(&pinned.id).expect("checks");
    assert_eq!(update.changed_files, vec!["SKILL.md".to_owned()]);
    let second = fixture.service.inspect(&candidate()).expect("re-inspects");
    assert_eq!(second.trust_state, "inspected");

    // Looking at the update changed nothing about what is installed.
    let still = fixture
        .service
        .installation(&pinned.id)
        .expect("still installed");
    assert!(still.skill_markdown.contains("# Demo skill"));
    assert_eq!(still.pinned_hash, pinned.pinned_hash);
    let applicable = fixture
        .service
        .applicable(&fixture.task_id, AdapterPolicy::Codex)
        .expect("applicable");
    assert_eq!(
        applicable.len(),
        1,
        "the pinned version still reaches agents"
    );

    // Installing the new version for the project pins that one separately.
    let project = fixture
        .service
        .trust_and_install(
            &second.id,
            Some(&fixture.repository_id),
            None,
            SkillScope::Project,
            true,
            false,
        )
        .expect("installs the update for the project");
    assert_ne!(project.install_path, pinned.install_path);
    assert!(project.skill_markdown.contains("# Changed"));
    let listed = fixture
        .service
        .list_for_task(&fixture.task_id)
        .expect("lists");
    assert_eq!(
        listed.len(),
        2,
        "task and project installations both reach the task"
    );
    let codex = fixture
        .service
        .applicable(&fixture.task_id, AdapterPolicy::Codex)
        .expect("applicable");
    assert_eq!(
        codex.len(),
        1,
        "the project install was not allowed for Codex"
    );
}

#[test]
fn shared_files_outlive_one_uninstall_and_a_global_reinstall_does_not_duplicate() {
    let fixture = Fixture::new(valid_files());
    let inspected = fixture.service.inspect(&candidate()).expect("inspects");
    let task = fixture
        .service
        .trust_and_install(
            &inspected.id,
            Some(&fixture.repository_id),
            Some(&fixture.task_id),
            SkillScope::Task,
            true,
            true,
        )
        .expect("installs for the task");
    let global = fixture
        .service
        .trust_and_install(
            &inspected.id,
            Some(&fixture.repository_id),
            Some(&fixture.task_id),
            SkillScope::Global,
            true,
            true,
        )
        .expect("installs globally");
    assert_eq!(
        task.install_path, global.install_path,
        "one copy per version"
    );
    assert_eq!(global.repository_id, None);
    assert_eq!(global.task_id, None);
    let again = fixture
        .service
        .trust_and_install(&inspected.id, None, None, SkillScope::Global, false, true)
        .expect("reinstalls globally");
    assert_eq!(again.id, global.id, "the same installation, updated");
    assert!(!again.allow_claude);
    assert_eq!(
        fixture
            .service
            .list_for_repository(&fixture.repository_id)
            .expect("lists")
            .len(),
        2
    );

    fixture
        .service
        .remove(&task.id)
        .expect("removes the task install");
    assert!(
        global.install_path.join("SKILL.md").is_file(),
        "still used globally"
    );
    fixture
        .service
        .remove(&global.id)
        .expect("removes the global install");
    assert!(
        !global.install_path.exists(),
        "no installation uses it any more"
    );
}

#[test]
fn a_block_outlives_reinspection_and_cannot_be_switched_back_on() {
    let fixture = Fixture::new(valid_files());
    let inspected = fixture.service.inspect(&candidate()).expect("inspects");
    let installed = fixture
        .service
        .trust_and_install(
            &inspected.id,
            Some(&fixture.repository_id),
            Some(&fixture.task_id),
            SkillScope::Task,
            true,
            true,
        )
        .expect("installs");
    fixture.service.block(&inspected.id).expect("blocks");
    let mut changed = valid_files();
    changed[1].contents = "#!/bin/sh\nexit 1\n".into();
    fixture.registry.set_files(changed);
    let again = fixture.service.inspect(&candidate()).expect("re-inspects");
    assert_eq!(again.trust_state, "blocked");
    assert!(matches!(
        fixture.service.set_enabled(&installed.id, true),
        Err(SkillError::Blocked(_))
    ));
    assert!(
        fixture
            .service
            .applicable(&fixture.task_id, AdapterPolicy::Claude)
            .expect("applicable")
            .is_empty()
    );
}

#[test]
fn installations_can_be_switched_off_and_limited_to_one_agent() {
    let fixture = Fixture::new(valid_files());
    let inspected = fixture.service.inspect(&candidate()).expect("inspects");
    let installed = fixture
        .service
        .trust_and_install(
            &inspected.id,
            Some(&fixture.repository_id),
            Some(&fixture.task_id),
            SkillScope::Task,
            true,
            true,
        )
        .expect("installs");
    let off = fixture
        .service
        .set_enabled(&installed.id, false)
        .expect("switches off");
    assert!(!off.enabled);
    assert!(
        fixture
            .service
            .applicable(&fixture.task_id, AdapterPolicy::Claude)
            .expect("applicable")
            .is_empty()
    );
    fixture
        .service
        .set_enabled(&installed.id, true)
        .expect("switches on");
    let claude_only = fixture
        .service
        .set_adapter_allowed(&installed.id, AdapterPolicy::Codex, false)
        .expect("limits");
    assert!(claude_only.allow_claude && !claude_only.allow_codex);
    assert!(
        fixture
            .service
            .applicable(&fixture.task_id, AdapterPolicy::Codex)
            .expect("applicable")
            .is_empty()
    );
    assert_eq!(
        fixture
            .service
            .applicable(&fixture.task_id, AdapterPolicy::Claude)
            .expect("applicable")
            .len(),
        1
    );
}

#[test]
fn skills_already_on_disk_are_found_with_who_loads_them() {
    let directory = TestDirectory::new();
    let repository = directory.path().join("repo");
    let home = directory.path().join("home");
    for (folder, name, description) in [
        (
            repository.join(".claude/skills/lint"),
            "lint",
            "Run the linters",
        ),
        (repository.join("skills/style"), "style", "House style"),
        (home.join(".codex/skills/notes"), "notes", "Keep notes"),
        (
            home.join(".codex/skills/.system/hidden"),
            "hidden",
            "Built in",
        ),
    ] {
        fs::create_dir_all(&folder).expect("folder");
        fs::write(
            folder.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: \"{description}\"\n---\n# {name}\n"),
        )
        .expect("skill");
    }
    let found = discover_local(&repository, Some(&home));
    let summary: Vec<(String, LocalOrigin, Vec<&str>)> = found
        .iter()
        .map(|skill| (skill.name.clone(), skill.origin, skill.loaded_by.clone()))
        .collect();
    assert_eq!(
        summary,
        vec![
            (
                "lint".to_owned(),
                LocalOrigin::RepositoryClaude,
                vec!["claude"]
            ),
            ("style".to_owned(), LocalOrigin::RepositoryShared, vec![]),
            ("notes".to_owned(), LocalOrigin::UserCodex, vec!["codex"]),
        ]
    );
    assert_eq!(found[0].description, "Run the linters");
    assert_eq!(front_matter_field("no front matter", "name"), None);
}

#[test]
#[ignore = "requires network access to skills.sh and explicitly installs a public skill snapshot"]
fn real_skills_sh_discovery_inspection_and_global_install() {
    let directory = TestDirectory::new();
    let storage = Arc::new(
        Storage::open(directory.path().join("mars.sqlite3")).expect("storage should open"),
    );
    let registry = Arc::new(SkillsShRegistry::new().expect("skills.sh client should build"));
    let service = SkillService::new(storage, registry, directory.path().join("installed"));
    let candidates = service
        .search("web design guidelines")
        .expect("skills.sh search should succeed");
    let candidate = candidates
        .into_iter()
        .find(|candidate| candidate.name == "web-design-guidelines")
        .expect("known public skill should be discoverable");
    let inspected = service
        .inspect(&candidate)
        .expect("downloaded skill should hash and expose SKILL.md");
    assert!(
        inspected
            .skill_markdown
            .contains("Web Interface Guidelines")
    );
    assert!(
        inspected.registry_verified,
        "the files should hash to the digest skills.sh publishes"
    );
    let installed = service
        .trust_and_install(&inspected.id, None, None, SkillScope::Global, true, true)
        .expect("inspected public skill should install");
    assert_eq!(
        fs::read_to_string(installed.install_path.join("SKILL.md"))
            .expect("installed SKILL.md should reopen"),
        inspected.skill_markdown,
    );
    service
        .remove(&installed.id)
        .expect("installed skill should remove");
    assert!(!installed.install_path.exists());
}

fn candidate() -> SkillCandidate {
    SkillCandidate {
        name: "demo".into(),
        slug: "mars/demo".into(),
        source: "mars/skills".into(),
        installs: 42,
    }
}

fn valid_files() -> Vec<SkillFile> {
    vec![
        SkillFile {
            path: "SKILL.md".into(),
            contents: "---\nname: demo\ndescription: Test skill\n---\n# Demo skill\nInspect me.\n"
                .into(),
        },
        SkillFile {
            path: "scripts/check.sh".into(),
            contents: "#!/bin/sh\nexit 0\n".into(),
        },
    ]
}

#[derive(Clone)]
struct FakeRegistry {
    snapshot: Arc<Mutex<SkillSnapshot>>,
}

impl FakeRegistry {
    fn new(files: Vec<SkillFile>, hash: Option<&str>) -> Self {
        let computed = registry_digest(&files);
        Self {
            snapshot: Arc::new(Mutex::new(SkillSnapshot {
                files,
                hash: hash.unwrap_or(&computed).to_owned(),
            })),
        }
    }

    fn set_files(&self, files: Vec<SkillFile>) {
        let hash = registry_digest(&files);
        *self.snapshot.lock().expect("registry lock should work") = SkillSnapshot { files, hash };
    }
}

impl RegistryClient for FakeRegistry {
    fn search(&self, _query: &str) -> Result<Vec<SkillCandidate>, SkillError> {
        Ok(vec![candidate()])
    }

    fn download(&self, _source: &str, _slug: &str) -> Result<SkillSnapshot, SkillError> {
        Ok(self
            .snapshot
            .lock()
            .expect("registry lock should work")
            .clone())
    }
}

struct Fixture {
    // Declaration order is drop order: the database and the service holding it
    // must close before the directory they live in is removed, or Windows
    // refuses the removal and the panic in `Drop` aborts the whole binary.
    service: SkillService,
    storage: Arc<Storage>,
    repository_id: String,
    task_id: String,
    install_root: PathBuf,
    registry: FakeRegistry,
    _directory: TestDirectory,
}

impl Fixture {
    fn new(files: Vec<SkillFile>) -> Self {
        Self::new_with_optional_hash(files, None)
    }

    fn new_with_hash(files: Vec<SkillFile>, hash: &str) -> Self {
        Self::new_with_optional_hash(files, Some(hash))
    }

    fn new_with_optional_hash(files: Vec<SkillFile>, hash: Option<&str>) -> Self {
        let directory = TestDirectory::new();
        let repository_root = directory.path().join("repository");
        fs::create_dir_all(&repository_root).expect("repository should exist");
        git(
            &repository_root,
            &["init", "--quiet", "--initial-branch=main"],
        );
        fs::write(repository_root.join("README.md"), "# fixture\n").expect("fixture should write");
        git(&repository_root, &["add", "README.md"]);
        git(
            &repository_root,
            &[
                "-c",
                "user.name=Mars Test",
                "-c",
                "user.email=mars@example.test",
                "commit",
                "--quiet",
                "-m",
                "initial",
            ],
        );
        let storage = Arc::new(
            Storage::open(directory.path().join("mars.sqlite3")).expect("storage should open"),
        );
        let OpenRepositoryOutcome::Opened(project) = ProjectService::new(storage.clone())
            .open_repository(&repository_root, TrustDecision::Trust)
            .expect("repository should open")
        else {
            panic!("trusted repository should open")
        };
        let task_id = TaskWorktreeService::new(storage.clone(), directory.path().join("worktrees"))
            .create_task(&project.repository_id, "Skill task", "use skill", "HEAD")
            .expect("task should plan")
            .task_id;
        let registry = FakeRegistry::new(files, hash);
        let install_root = directory.path().join("installed-skills");
        let service = SkillService::new(storage.clone(), Arc::new(registry.clone()), &install_root);
        Self {
            service,
            storage,
            repository_id: project.repository_id,
            task_id,
            install_root,
            registry,
            _directory: directory,
        }
    }
}

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .expect("Git should launch");
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir()
            .join(format!("mars-skill-fixture-{}-{nonce}", std::process::id(),));
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
