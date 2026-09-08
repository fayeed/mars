//! CLI harness for Project Brain.
//!
//! Every command drives the same typed API the desktop and the agent tool use,
//! so the harness exercises real behaviour rather than a parallel
//! implementation. The answer document is JSON on stdout; streamed pipeline
//! events go to stderr, which is what makes the streaming contract observable
//! without a UI.
//!
//! ```text
//! mars-brain --repo <path> ask <question...> [--budget N] [--max-sources N] [--stream]
//! mars-brain --repo <path> explain <symbol-id>
//! mars-brain --repo <path> where-is <concept...>
//! mars-brain --repo <path> dependencies <path-or-symbol-id>
//! mars-brain --repo <path> prompt <question...>
//! mars-brain --repo <path> memory-pending
//! mars-brain --repo <path> memory-list [--kind K] [--scope S] [--contains TEXT]
//! mars-brain --repo <path> memory-usage <memory-id>
//! ```
//!
//! The memory commands are read-only on purpose. Proposing needs a
//! conversation to quote and approving needs a person, and neither belongs to
//! a shell script: an approval typed by a script would be exactly the
//! rubber-stamp the design exists to prevent.
//!
//! `prompt` renders the exact synthesis prompt for a question without calling a
//! provider, so what a model would be asked can be inspected directly.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use mars_brain::{
    AskRequest, BrainEvent, ClaimKind, DEFAULT_BUDGET_TOKENS, DEFAULT_MAX_SOURCES, MemoryFilters,
    MemoryKind, MemoryScope, MemoryService, ProjectBrain, ProposalFilters,
};
use mars_core::projects::{OpenRepositoryOutcome, ProjectService};
use mars_indexer::api::IndexApi;
use mars_security::TrustDecision;
use mars_storage::Storage;
use tokio_util::sync::CancellationToken;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mars-brain: {error}");
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

    let index = Arc::new(IndexApi::new(storage.clone()));
    let brain = ProjectBrain::new(storage.clone(), index);
    let cancellation = CancellationToken::new();
    let stream = arguments.flag("stream").is_some();

    match arguments.command.as_str() {
        "ask" => {
            let question = arguments.joined_positional("ask needs a question")?;
            let mut request = AskRequest::new(&repository_id, question);
            request.budget_tokens = arguments.number("budget", DEFAULT_BUDGET_TOKENS)?;
            request.max_sources = arguments.number("max-sources", DEFAULT_MAX_SOURCES)?;
            let answer = brain
                .ask_observing(&request, &cancellation, |event| report(event, stream))
                .map_err(stringify)?;
            summarize(&answer);
            print(&answer);
        }
        "explain" => {
            let symbol_id = arguments
                .positional
                .first()
                .ok_or("explain needs a symbol id")?;
            let answer = brain
                .explain_observing(&repository_id, symbol_id, &cancellation, |event| {
                    report(event, stream);
                })
                .map_err(stringify)?;
            summarize(&answer);
            print(&answer);
        }
        "where-is" => {
            let concept = arguments.joined_positional("where-is needs a concept")?;
            let answer = brain
                .where_is_observing(&repository_id, &concept, &cancellation, |event| {
                    report(event, stream);
                })
                .map_err(stringify)?;
            summarize(&answer);
            print(&answer);
        }
        "dependencies" => {
            let target = arguments
                .positional
                .first()
                .ok_or("dependencies needs a file path or symbol id")?;
            let slice = brain
                .dependencies(&repository_id, target, &cancellation)
                .map_err(stringify)?;
            eprintln!(
                "{} node(s), {} edge(s), {}ms",
                slice.nodes.len(),
                slice.edges.len(),
                slice.latency_ms
            );
            print(&slice);
        }
        "prompt" => {
            let question = arguments.joined_positional("prompt needs a question")?;
            // Rendering the prompt is a read of the pipeline's own contract, so
            // it is built from the same instructions synthesis would use.
            let request = mars_brain::SynthesisRequest {
                instructions: mars_brain::SYNTHESIS_INSTRUCTIONS.to_owned(),
                question,
                sources: Vec::new(),
            };
            println!("{}", mars_brain::render_synthesis_prompt(&request));
        }
        "memory-pending" => {
            let (memory, project_id) = memory_service(&storage, &repository_id)?;
            let pending = memory
                .pending(&ProposalFilters {
                    project_id,
                    ..ProposalFilters::default()
                })
                .map_err(stringify)?;
            eprintln!("{} proposal(s) awaiting a human", pending.len());
            print(&pending);
        }
        "memory-list" => {
            let (memory, project_id) = memory_service(&storage, &repository_id)?;
            let memories = memory
                .list(&MemoryFilters {
                    project_id,
                    repository_id: Some(repository_id.clone()),
                    kinds: arguments
                        .flag("kind")
                        .map(|kind| parse_kind(&kind))
                        .transpose()?
                        .into_iter()
                        .collect(),
                    scopes: arguments
                        .flag("scope")
                        .map(|scope| parse_scope(&scope))
                        .transpose()?
                        .into_iter()
                        .collect(),
                    contains: arguments.flag("contains"),
                    ..MemoryFilters::default()
                })
                .map_err(stringify)?;
            for record in &memories {
                eprintln!(
                    "  [{}/{}] {} -- approved by {}, used in {} task(s)",
                    record.kind.as_str(),
                    record.scope.as_str(),
                    record.text,
                    record.approved_by,
                    record.used_in_tasks
                );
            }
            print(&memories);
        }
        "memory-usage" => {
            let memory_id = arguments
                .positional
                .first()
                .ok_or("memory-usage needs a memory id")?;
            let (memory, _) = memory_service(&storage, &repository_id)?;
            let usage = memory.usage(memory_id).map_err(stringify)?;
            eprintln!(
                "{} injection(s) across {} task(s)",
                usage.use_count, usage.used_in_tasks
            );
            print(&usage);
        }
        other => return Err(format!("unknown command `{other}`")),
    }
    Ok(())
}

fn memory_service(
    storage: &Arc<Storage>,
    repository_id: &str,
) -> Result<(MemoryService, String), String> {
    let project_id: String = storage
        .with_connection(|connection| {
            connection.query_row(
                "SELECT project_id FROM repositories WHERE id = ?1",
                [repository_id],
                |row| row.get(0),
            )
        })
        .map_err(stringify)?;
    Ok((MemoryService::new(storage.clone()), project_id))
}

fn parse_kind(value: &str) -> Result<MemoryKind, String> {
    match value {
        "convention" => Ok(MemoryKind::Convention),
        "decision" => Ok(MemoryKind::Decision),
        "preference" => Ok(MemoryKind::Preference),
        "fact" => Ok(MemoryKind::Fact),
        "caution" => Ok(MemoryKind::Caution),
        other => Err(format!(
            "unknown kind `{other}`; expected convention, decision, preference, fact, or caution"
        )),
    }
}

fn parse_scope(value: &str) -> Result<MemoryScope, String> {
    match value {
        "repo" => Ok(MemoryScope::Repo),
        "task" => Ok(MemoryScope::Task),
        "global" => Ok(MemoryScope::Global),
        other => Err(format!(
            "unknown scope `{other}`; expected repo, task, or global"
        )),
    }
}

/// Pipeline events on stderr, so stdout stays a single JSON document.
fn report(event: &BrainEvent, stream: bool) {
    if !stream {
        return;
    }
    match event {
        BrainEvent::Planned { queries, intent } => {
            eprintln!("planned {intent:?}: {}", queries.join(" | "));
        }
        BrainEvent::Retrieved {
            candidates,
            stale_paths,
        } => eprintln!("retrieved {candidates} candidate(s), {stale_paths} stale path(s)"),
        BrainEvent::Packed {
            sources,
            tokens,
            dropped,
        } => eprintln!("packed {sources} source(s), {tokens} token(s), dropped {dropped}"),
        BrainEvent::Synthesized { provider, claims } => {
            eprintln!("synthesized {claims} claim(s) via {provider}");
        }
        BrainEvent::Citation(citation) => eprintln!(
            "  [{}] {}:{}-{}",
            citation.id, citation.path, citation.line_start, citation.line_end
        ),
        BrainEvent::Claim(claim) => eprintln!(
            "  {} {} {}",
            claim.kind.label().to_uppercase(),
            claim.text,
            if claim.citations.is_empty() {
                format!("(from {})", claim.supports.join(", "))
            } else {
                format!("({})", claim.citations.join(", "))
            }
        ),
        BrainEvent::Gap(gap) => eprintln!("  GAP {:?}: {}", gap.kind, gap.subject),
        BrainEvent::Completed {
            sources,
            claims,
            latency_ms,
            confidence,
        } => eprintln!(
            "completed: {claims} claim(s), {sources} source(s), {latency_ms}ms, confidence {confidence:.2}"
        ),
        BrainEvent::Cancelled { stage } => eprintln!("cancelled during {stage}"),
    }
}

/// The one-line header a surface would show: the same figures, verbatim.
fn summarize(answer: &mars_brain::BrainAnswer) {
    let facts = answer
        .claims
        .iter()
        .filter(|claim| claim.kind == ClaimKind::Fact)
        .count();
    eprintln!(
        "{:.1}s · {} sources · {facts} fact(s), {} inference(s) · {} gap(s)",
        answer.latency_ms as f64 / 1000.0,
        answer.sources,
        answer.claims.len() - facts,
        answer.gaps.len()
    );
}

impl Arguments {
    fn flag(&self, name: &str) -> Option<String> {
        self.flags
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    }

    fn number(&self, name: &str, fallback: usize) -> Result<usize, String> {
        match self.flag(name) {
            Some(value) => value
                .parse()
                .map_err(|_| format!("`--{name}` needs a whole number, not `{value}`")),
            None => Ok(fallback),
        }
    }

    /// Questions are typed as ordinary words, so the positional arguments are
    /// the question rather than one quoted string.
    fn joined_positional(&self, message: &str) -> Result<String, String> {
        if self.positional.is_empty() {
            return Err(message.to_owned());
        }
        Ok(self.positional.join(" "))
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
            "--stream" => flags.push(("stream".into(), "true".into())),
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

/// The harness keeps its database outside the repository being read, in the
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
        Err(error) => eprintln!("mars-brain: could not encode output: {error}"),
    }
}

fn stringify(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn usage() -> String {
    let name = Path::new(file!())
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("mars-brain");
    format!(
        "usage: {name} --repo <path> \n<ask|explain|where-is|dependencies|prompt|memory-pending|memory-list|memory-usage> [arguments]"
    )
}
