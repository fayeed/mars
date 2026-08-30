use mars_security::{is_default_ignored_directory, is_sensitive_filename};

#[test]
fn build_dependency_and_vcs_directories_are_denied_by_default() {
    for name in [".git", "node_modules", "target", "dist", "build", ".next"] {
        assert!(
            is_default_ignored_directory(name),
            "{name} should be ignored"
        );
    }
    assert!(!is_default_ignored_directory("src"));
}

#[test]
fn credential_files_are_denied_but_examples_remain_visible() {
    for name in [
        ".env",
        ".env.local",
        "id_rsa",
        "credentials.json",
        ".npmrc",
        "certificate.p12",
        "private.pem",
    ] {
        assert!(is_sensitive_filename(name), "{name} should be sensitive");
    }
    assert!(!is_sensitive_filename(".env.example"));
    assert!(!is_sensitive_filename("src.ts"));
}
