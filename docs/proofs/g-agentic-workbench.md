# Phase 4 proof — the agentic workbench

Date: 2026-10-09
Branch: `main`
Status: every deterministic gate passes; two live checks are left to the person (below)

## What Phase 4 set out to do

Make Mars something to build software in with agents, end to end:

1. a project graph and knowledge base agents query instead of re-reading files with `grep`;
2. skills found and installed like editor extensions, loaded natively;
3. several agents at once, shown together;
4. chat first;
5. worktrees;
6. Git and GitHub without leaving the window;
7. first-class memory;
8. a real editor when a person wants to change code themselves;
9. a place to see and settle what agents decided.

## Scope implemented

| Need | Where it lives | ADR |
| --- | --- | --- |
| Graph for agents | `crates/mcp`: 18 tools over stdio, served by `mars-desktop mcp` and `mars-mcp`; given to every run through `--mcp-config` (Claude Code) and `-c mcp_servers.mars.*` (Codex) | 046 |
| Graph for people | Knowledge → Code graph: used-by / uses columns, re-centre, explain, blast radius, why, where-is | 055 |
| Fresh index | Index on first open; `RepositoryWatcher` re-reads saves and branch switches; build and dependency output never scheduled | 055 |
| Skills | Marketplace (Discover, Installed, On this machine), pinned content-addressed installs verified against the skills.sh digest, Claude Code loads them as a generated `--plugin-dir` plugin | 051 |
| Several agents | Agents destination: every run on a shared clock, waiting-on-you first, Stop; rail and status bar from the same live set | 050 |
| Chat first | A task's conversation is one provider session (`--resume`, `exec resume`); follow-ups queue behind a working run | 045 |
| Asking before acting | Claude's `--permission-prompt-tool` is Mars; out-of-scope effects wait for a person in the conversation, across processes | 047 |
| Worktrees | Plans, provisioning, drift, cleanup and restore (Phase 3), now with Git for every worktree | 040–041, 052 |
| Git | Status, stage, commit, history, branches, sync, stash for the repository and each worktree; merge home through a `merge-tree` preview | 052 |
| GitHub | Issues and pull requests with conversations, checks and line comments; comment and merge confirmed against the account; issue → planned task; `gh` sign-in reuse | 053 |
| Memory | Project-wide proposal queue with conflict settlement; agents propose through `remember`, read through `memory` | 054 |
| Editor | Monaco with versioned saves, files opened in the task's own worktree | — |
| Decisions | Knowledge → Decisions: every decision, constraint, risk and question with agent, task, alternatives and files; approve, reword, reject, retire, pin; counted on the rail; cards in the transcript | 054 |

Agent CLIs are found in Settings, on `PATH`, at installer locations, or inside editor extensions (ADR-048), and every child process starts without a console window on Windows.

## Deterministic evidence

Selected tests, each against real components (SQLite, system Git, child processes) rather than mocks:

- `crates/mcp/tests/server.rs` — initialise and list over stdio; a reviewer gets no write; graph tools answer from a real index; an unindexed repository says so; a recorded decision is a candidate on its task; a memory proposal is verified against the quoted turn; the permission prompt allows granted effects, refuses escapes, and waits for and carries a person's answer.
- `crates/agents/tests/streams.rs` — tool results pair with calls and reasoning is dropped (Claude and Codex); prompts on stdin; Codex keeps its sandbox on resume; executables found through `PATHEXT`, installers and editor extensions; the pump delivers whole characters, exactly one ending, and recovers dropped live events.
- `crates/skills/tests/install.rs` — inspection before install; pins survive re-inspection; quarantine of unsafe or mismatched downloads; the registry digest matches skills.sh's scheme; shared store files outlive one uninstall; blocks persist.
- `crates/git/tests/scm.rs` — status, staging, discard, branches, stash, push/fetch/pull against a real remote; merge, squash and fast-forward home exactly as previewed; a conflict is predicted, stops, and leaves the base untouched; stale previews and dirty bases are refused.
- `crates/forge-github/tests/api.rs` — issue lists exclude pull requests; comments read and post; a merge is pinned to the head that was reviewed.
- `crates/brain/tests/knowledge.rs` — a record's origin (task, agent, alternatives, files) survives a correction.
- `crates/indexer/src/watcher.rs` — only paths an index pass would read are scheduled.
- Desktop: `GitHubPanel`, `DecisionLog`, `KnowledgeView`, `AgentKnowledgeEvent`, `CodeGraph`, `MemoryBrowser` conflicts, `ActivityLog` focus, `CompletionSummary` decisions, `SourceControl`, `AgentsBoard`, `SkillsMarketplace`, `GitPanel`, `IntegratePanel`, `Composer`, and the shell's five destinations and rail badge.

The full gate passes on Windows:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace            309 passed, 0 failed, 5 ignored
pnpm -r typecheck
pnpm lint
pnpm test                         67 files, 497 tests passed
pnpm build:web
pnpm check:protocol
git diff --check
```

The five ignored tests need a real CLI or the network. Four were run on this machine and pass: Claude Code and Codex reporting their sign-in (using the copies inside the VS Code extensions), Codex parsing the MCP overrides Mars builds, and discovery, inspection and install against the live skills.sh registry. The fifth, a Codex memory extraction, is a paid model run and was not.

## Not claimed

- **A live agent run.** No test starts a real Claude Code or Codex run, because one spends the person's plan. The streams, resume commands, MCP configuration and permission hook are proven against recorded frames, fixture agents and the real CLIs' argument parsers; the end-to-end check is in [running](../runbooks/running.md#checking-a-real-run) and should be done once before a release.
- **Live GitHub writes.** Listing, commenting and merging are proven against a transport double that checks each request's route and body, including the head a merge is pinned to. No comment or merge was posted to a real repository.
