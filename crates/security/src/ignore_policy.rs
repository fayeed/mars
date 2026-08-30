const DEFAULT_IGNORED_DIRECTORIES: [&str; 6] =
    [".git", "node_modules", "target", "dist", "build", ".next"];

#[must_use]
pub fn is_default_ignored_directory(name: &str) -> bool {
    DEFAULT_IGNORED_DIRECTORIES.contains(&name)
}

#[must_use]
pub fn is_sensitive_filename(name: &str) -> bool {
    ((name == ".env" || name.starts_with(".env.")) && name != ".env.example")
        || matches!(
            name,
            "id_rsa" | "id_ed25519" | "credentials" | "credentials.json" | ".npmrc" | ".pypirc"
        )
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with(".p12")
        || name.ends_with(".pfx")
}
