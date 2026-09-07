//! CLI harness for the index service.
//!
//! Every command drives the same typed API the desktop uses, so the harness
//! exercises real behaviour rather than a parallel implementation. Output is
//! JSON on stdout; diagnostics go to stderr.
//!
//! ```text
//! mars-index --repo <path> status
//! mars-index --repo <path> reindex [paths...]
//! mars-index --repo <path> search <lexical|semantic|hybrid> <query> [--k N] [--path P] [--lang L]
//! mars-index --repo <path> symbols-at <path> <line>
//! mars-index --repo <path> graph <symbol-id> [--depth N] [--kind K] [--direction in|out|both]
//! mars-index --repo <path> depends <path>
//! mars-index --repo <path> blame <path> <start> <end>
//! mars-index --repo <path> ignored <path>
//! mars-index --repo <path> stale
//! mars-index --repo <path> watch [--seconds N]
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_indexer::api::{Direction, IndexApi, NeighborOptions};
use mars_indexer::search::SearchFilters;
use mars_indexer::watcher::{RepositoryWatcher, WatchOptions};
use mars_indexer::{IndexEvent, walk};
use mars_security::TrustDecision;
use mars_storage::Storage;
use tokio_util::sync::CancellationToken;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mars-index: {error}");
            ExitCode::FAILURE
        }
    }
}

struct Arguments {
    repository: PathBuf,
    database: Option<PathBuf>,
    command: String,
    positional: Vec<String>,
    flags: Vec<(String, String)>,
}

fn run() -> Result<(), String> {
    let arguments = parse_arguments()?;
    let database = arguments.database.clone().unwrap_or_else(default_database);
    if let Some(parent) = database.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let storage = Arc::new(Storage::open(&database).map_err(|error| error.to_string())?);

    let root = arguments
        .repository
        .canonicalize()
        .map_err(|error| format!("{}: {error}", arguments.repository.display()))?;
    let outcome = ProjectService::new(storage.clone())
        .open_repository(&root, TrustDecision::Trust)
        .map_err(|error| error.to_string())?;
    let OpenRepositoryOutcome::Opened(project) = outcome else {
        return Err("repository requires trust confirmation".into());
    };
    let repository_id = project.repository_id;
    let api = IndexApi::new(storage.clone());

    match arguments.command.as_str() {
        "status" => print(&api.status(&repository_id).map_err(stringify)?),
        "reindex" => {
            let paths = (!arguments.positional.is_empty()).then(|| arguments.positional.clone());
            let started = Instant::now();
            let quiet = arguments.flag("quiet").is_some();
            let summary = api
                .reindex_observing(&repository_id, paths, &CancellationToken::new(), |event| {
                    if quiet {
                        return;
                    }
                    // Progress is a stream on stderr so stdout stays a single
                    // JSON document.
                    if let IndexEvent::FileIndexed {
                        relative_path,
                        position,
                        total_files,
                        ..
                    } = event
                    {
                        eprintln!("[{position}/{total_files}] {relative_path}");
                    }
                })
                .map_err(stringify)?;
            eprintln!("indexed in {} ms", started.elapsed().as_millis());
            print(&summary);
        }
        "search" => {
            let mode = arguments
                .positional
                .first()
                .map(String::as_str)
                .unwrap_or("hybrid");
            let query = arguments
                .positional
                .get(1..)
                .map(|rest| rest.join(" "))
                .unwrap_or_default();
            if query.trim().is_empty() {
                return Err("search needs a query".into());
            }
            let limit = arguments
                .flag("k")
                .and_then(|value| value.parse().ok())
                .unwrap_or(10);
            let filters = SearchFilters {
                path_prefixes: arguments.flags_all("path"),
                languages: arguments.flags_all("lang"),
            };
            let started = Instant::now();
            let results = match mode {
                "lexical" => api.search_lexical(&repository_id, &query, &filters, limit),
                "semantic" => api.search_semantic(&repository_id, &query, limit, &filters),
                "hybrid" => api.search_hybrid(&repository_id, &query, limit, &filters),
                other => return Err(format!("unknown search mode `{other}`")),
            }
            .map_err(stringify)?;
            eprintln!("searched in {} ms", started.elapsed().as_millis());
            print(&results);
        }
        "symbols-at" => {
            let path = arguments
                .positional
                .first()
                .ok_or("symbols-at needs a path")?;
            let line = arguments
                .positional
                .get(1)
                .and_then(|value| value.parse().ok())
                .ok_or("symbols-at needs a line number")?;
            print(
                &api.symbols_at(&repository_id, path, line)
                    .map_err(stringify)?,
            );
        }
        "graph" => {
            let symbol_id = arguments
                .positional
                .first()
                .ok_or("graph needs a symbol id")?;
            let options = NeighborOptions {
                kinds: arguments.flags_all("kind"),
                depth: arguments
                    .flag("depth")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1),
                direction: match arguments.flag("direction").as_deref() {
                    Some("in") => Direction::Incoming,
                    Some("out") => Direction::Outgoing,
                    _ => Direction::Both,
                },
                limit: arguments
                    .flag("limit")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(100),
            };
            let started = Instant::now();
            let neighborhood = api
                .graph_neighbors(&repository_id, symbol_id, &options)
                .map_err(stringify)?;
            eprintln!("traversed in {} ms", started.elapsed().as_millis());
            print(&neighborhood);
        }
        "depends" => {
            let path = arguments.positional.first().ok_or("depends needs a path")?;
            print(
                &api.file_dependencies(&repository_id, path)
                    .map_err(stringify)?,
            );
        }
        "blame" => {
            let path = arguments.positional.first().ok_or("blame needs a path")?;
            let start = arguments
                .positional
                .get(1)
                .and_then(|value| value.parse().ok())
                .unwrap_or(1);
            let end = arguments
                .positional
                .get(2)
                .and_then(|value| value.parse().ok())
                .unwrap_or(start);
            print(
                &api.history_blame(&repository_id, path, start, end)
                    .map_err(stringify)?,
            );
        }
        "ignored" => {
            let path = arguments.positional.first().ok_or("ignored needs a path")?;
            print(&api.ignore_status(&repository_id, path).map_err(stringify)?);
        }
        "stale" => print(&api.stale_paths(&repository_id).map_err(stringify)?),
        "files" => {
            let outcome = walk::walk(&root).map_err(stringify)?;
            print(&serde_json::json!({
                "files": outcome.files.len(),
                "ignored": outcome.ignored.len(),
            }));
        }
        "watch" => {
            let seconds = arguments
                .flag("seconds")
                .and_then(|value| value.parse().ok())
                .unwrap_or(30_u64);
            eprintln!("watching {} for {seconds}s", root.display());
            let watcher = RepositoryWatcher::start(
                storage.clone(),
                &repository_id,
                &root,
                WatchOptions::default(),
                |event| match serde_json::to_string(event) {
                    Ok(line) => println!("{line}"),
                    Err(error) => eprintln!("watch event could not be encoded: {error}"),
                },
            )
            .map_err(stringify)?;
            std::thread::sleep(Duration::from_secs(seconds));
            watcher.stop();
        }
        other => return Err(format!("unknown command `{other}`")),
    }
    Ok(())
}

impl Arguments {
    fn flag(&self, name: &str) -> Option<String> {
        self.flags
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    }

    fn flags_all(&self, name: &str) -> Vec<String> {
        self.flags
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
            .collect()
    }
}

fn parse_arguments() -> Result<Arguments, String> {
    let mut repository = None;
    let mut database = None;
    let mut command = None;
    let mut positional = Vec::new();
    let mut flags = Vec::new();

    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--repo" | "-C" => repository = arguments.next().map(PathBuf::from),
            "--database" => database = arguments.next().map(PathBuf::from),
            "--help" | "-h" => {
                println!("{}", usage());
                std::process::exit(0);
            }
            // A bare `--quiet` is a switch; every other flag takes a value.
            "--quiet" => flags.push(("quiet".into(), "true".into())),
            value if value.starts_with("--") => {
                let key = value.trim_start_matches("--").to_owned();
                let value = arguments
                    .next()
                    .ok_or_else(|| format!("flag `--{key}` needs a value"))?;
                flags.push((key, value));
            }
            value if command.is_none() => command = Some(value.to_owned()),
            value => positional.push(value.to_owned()),
        }
    }

    Ok(Arguments {
        repository: repository.unwrap_or_else(|| PathBuf::from(".")),
        database,
        command: command.ok_or_else(usage)?,
        positional,
        flags,
    })
}

/// The harness keeps its database outside the repository being indexed, in the
/// same place the desktop profile would.
fn default_database() -> PathBuf {
    let base = std::env::var_os("MARS_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local/share"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("dev.mars.desktop").join("mars.sqlite3")
}

fn print<T: serde::Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!("mars-index: could not encode output: {error}"),
    }
}

fn stringify(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn usage() -> String {
    let name = Path::new(file!())
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("mars-index");
    format!(
        "usage: {name} --repo <path> <status|reindex|search|symbols-at|graph|depends|blame|ignored|stale|files|watch> [arguments]"
    )
}
