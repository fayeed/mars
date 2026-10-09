//! Child processes started by a windowed app.

use std::ffi::OsStr;
use std::process::Command;

/// A command for a console program that will not open a console window.
///
/// A release build of the desktop app is a GUI program with no console of its
/// own, so on Windows every console program it starts — Git, the GitHub CLI,
/// an agent CLI — would otherwise flash a window of its own, once per call.
/// Output is still read through pipes. On other platforms this is exactly
/// `Command::new`.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    hide_window(&mut command);
    command
}

/// Keep a command that was built elsewhere from opening a console window.
pub fn hide_window(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::command;

    #[test]
    fn a_hidden_command_still_runs_and_returns_its_output() {
        let output = command("git").arg("--version").output().expect("git runs");
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).starts_with("git version"));
    }
}
