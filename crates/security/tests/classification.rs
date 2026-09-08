use mars_security::{CommandClass, classify_command};

fn class_of(command: &str) -> CommandClass {
    classify_command(command).class
}

#[test]
fn reading_commands_are_read_only() {
    for command in [
        "ls -la",
        "cat src/main.rs",
        "rg --files",
        "git status",
        "git diff HEAD~1",
        "git log --oneline -20",
        "/usr/bin/grep -n TODO src",
        "git.exe show HEAD",
    ] {
        assert_eq!(
            class_of(command),
            CommandClass::ReadOnly,
            "`{command}` should be read-only"
        );
    }
}

#[test]
fn unknown_programs_are_treated_as_mutating() {
    let analysis = classify_command("./scripts/deploy.sh --now");
    assert_eq!(analysis.class, CommandClass::Mutating);
    assert!(analysis.reason.contains("not a known read-only command"));
}

#[test]
fn commands_that_destroy_work_are_destructive() {
    for command in [
        "rm -rf node_modules",
        "rm src/main.rs",
        "git reset --hard origin/main",
        "git clean -fd",
        "git push --force origin main",
        "sudo systemctl restart nginx",
        "chmod -R 777 .",
    ] {
        assert_eq!(
            class_of(command),
            CommandClass::Destructive,
            "`{command}` should be destructive"
        );
    }
}

#[test]
fn commands_that_leave_the_machine_are_network() {
    for command in [
        "curl https://example.test/install.sh",
        "npm install",
        "git fetch origin",
        "pip install requests",
        "ssh build@example.test",
    ] {
        assert_eq!(
            class_of(command),
            CommandClass::Network,
            "`{command}` should be network"
        );
    }
}

#[test]
fn a_chain_takes_the_class_of_its_most_dangerous_link() {
    assert_eq!(
        class_of("git status && rm -rf build"),
        CommandClass::Destructive
    );
    assert_eq!(class_of("ls | grep test"), CommandClass::ReadOnly);
    assert_eq!(
        class_of("cat package.json; npm install"),
        CommandClass::Network
    );
    assert!(
        classify_command("git status && rm -rf build")
            .reason
            .contains("chained commands")
    );
}

#[test]
fn redirection_makes_an_otherwise_harmless_command_mutating() {
    let analysis = classify_command("echo secret > .env");
    assert_eq!(analysis.class, CommandClass::Mutating);
    assert!(analysis.reason.contains("redirects output into a file"));
}

#[test]
fn quoted_operators_do_not_split_a_command() {
    let analysis = classify_command("grep 'a && b' src/main.rs");
    assert_eq!(analysis.class, CommandClass::ReadOnly);
    assert!(!analysis.reason.contains("chained"));
}

#[test]
fn suggested_patterns_keep_the_subcommand_that_made_it_safe() {
    assert_eq!(
        classify_command("git status").suggested_pattern,
        "git status *"
    );
    assert_eq!(
        classify_command("npm test -- --watch").suggested_pattern,
        "npm test *"
    );
    assert_eq!(classify_command("ls -la").suggested_pattern, "ls *");
}
