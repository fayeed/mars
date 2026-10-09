//! A stand-in agent CLI for tests that need a real process on every platform.
//!
//! ```text
//! mars-agent-fixture <script.jsonl> [--exit N] [--echo-stdin]
//! ```
//!
//! Reads its whole standard input first, the way `claude -p` does. With
//! `--echo-stdin` it then reports that input back as one Claude-shaped
//! assistant message, so a test can see the prompt arrived intact. Each line of
//! the script is written and flushed on its own. A line `#split` writes the line
//! after it in two halves with a pause between, cutting a multi-byte character
//! in two when there is one, so readers that decode per read are caught out.
//! A line `#sleep <ms>` pauses.

use std::io::{Read, Write};
use std::time::Duration;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let script = arguments.first().cloned().unwrap_or_default();
    let exit_code = arguments
        .iter()
        .position(|argument| argument == "--exit")
        .and_then(|index| arguments.get(index + 1))
        .and_then(|code| code.parse::<i32>().ok())
        .unwrap_or(0);
    let echo = arguments.iter().any(|argument| argument == "--echo-stdin");

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if echo {
        let line = serde_json::json!({
            "type": "assistant",
            "message": { "content": [{ "type": "text", "text": input }] }
        });
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    let contents = std::fs::read_to_string(&script).unwrap_or_default();
    let mut split_next = false;
    for line in contents.lines() {
        if line == "#split" {
            split_next = true;
            continue;
        }
        if let Some(milliseconds) = line.strip_prefix("#sleep ") {
            let milliseconds = milliseconds.trim().parse().unwrap_or(0);
            std::thread::sleep(Duration::from_millis(milliseconds));
            continue;
        }
        let bytes = format!("{line}\n").into_bytes();
        if split_next {
            split_next = false;
            // Cut inside the first multi-byte character, or in the middle.
            let cut = bytes
                .iter()
                .position(|byte| *byte >= 0x80)
                .map_or(bytes.len() / 2, |index| index + 1);
            let _ = out.write_all(&bytes[..cut]);
            let _ = out.flush();
            std::thread::sleep(Duration::from_millis(60));
            let _ = out.write_all(&bytes[cut..]);
        } else {
            let _ = out.write_all(&bytes);
        }
        let _ = out.flush();
    }
    drop(out);
    std::process::exit(exit_code);
}
