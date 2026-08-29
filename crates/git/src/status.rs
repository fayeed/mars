use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredWorktree {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub prunable: bool,
}

#[must_use]
pub fn status_is_dirty(porcelain_v1_z: &[u8]) -> bool {
    !porcelain_v1_z.is_empty()
}

#[must_use]
pub fn parse_worktree_porcelain_z(output: &[u8]) -> Vec<RegisteredWorktree> {
    let mut records = Vec::new();
    let mut current: Option<RegisteredWorktree> = None;
    for raw in output.split(|byte| *byte == 0) {
        if raw.is_empty() {
            if let Some(record) = current.take() {
                records.push(record);
            }
            continue;
        }
        let field = String::from_utf8_lossy(raw);
        if let Some(path) = field.strip_prefix("worktree ") {
            if let Some(record) = current.replace(RegisteredWorktree {
                path: PathBuf::from(path),
                head: None,
                branch: None,
                prunable: false,
            }) {
                records.push(record);
            }
        } else if let Some(record) = current.as_mut() {
            if let Some(head) = field.strip_prefix("HEAD ") {
                record.head = Some(head.to_owned());
            } else if let Some(branch) = field.strip_prefix("branch ") {
                record.branch = Some(branch.to_owned());
            } else if field == "prunable" || field.starts_with("prunable ") {
                record.prunable = true;
            }
        }
    }
    if let Some(record) = current {
        records.push(record);
    }
    records
}

#[must_use]
pub fn is_registered(worktrees: &[RegisteredWorktree], path: &Path) -> bool {
    worktrees.iter().any(|worktree| worktree.path == path)
}

#[cfg(test)]
mod tests {
    use super::{parse_worktree_porcelain_z, status_is_dirty};

    #[test]
    fn parses_nul_delimited_worktrees_and_dirty_status() {
        let records = parse_worktree_porcelain_z(
            b"worktree /tmp/main\0HEAD abc\0branch refs/heads/main\0\0worktree /tmp/with spaces\0HEAD def\0prunable reason\0\0",
        );
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].path.to_string_lossy(), "/tmp/with spaces");
        assert!(records[1].prunable);
        assert!(!status_is_dirty(b""));
        assert!(status_is_dirty(b" M src/main.ts\0"));
    }
}
