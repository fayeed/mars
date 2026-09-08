import type {
  CommandFailure,
  ContextItem,
  FileDocument,
  FilePage,
  IndexStatus,
  ProjectSnapshot,
  RunContextPack,
  SetupCommandInput,
  SkillInstallation,
  StartingPoint,
  WorktreePlan,
  WorktreeProposal,
  WorktreeState,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { lazy, Suspense, useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { RunContextReview } from "@/features/agents/RunContextReview";
import { BrainAnswer } from "@/features/brain/BrainAnswer";
import { FileExplorer } from "@/features/explorer/FileExplorer";
import { RepositoryHome, indexActivityState } from "@/features/home/RepositoryHome";
import { CreateTaskDialog } from "@/features/tasks/CreateTaskDialog";
import { TaskDetail } from "@/features/tasks/TaskDetail";
import { WorktreePlanMessage, type PlanEdits } from "@/features/tasks/WorktreePlanMessage";
import { TerminalPanel } from "@/features/terminal/TerminalPanel";
import type { ActivityState } from "@/shell/activity-state";
import { Composer } from "@/shell/Composer";
import type {
  ComposerAgent,
  ComposerAttachment,
  ComposerScope,
  MentionTarget,
  ShellTask,
} from "@/shell/model";
import { MetaLabel, Mono, StateBlock, StateLabel } from "@/shell/primitives";
import { ShellFrame } from "@/shell/ShellFrame";
import { useShell } from "@/shell/shell-store";

const CodeEditor = lazy(() =>
  import("@/features/editor/CodeEditor").then((module) => ({ default: module.CodeEditor })),
);

interface ShellWorkspaceProps {
  connection: ReactNode;
  onOpenProject: (project: ProjectSnapshot) => void;
  project: ProjectSnapshot;
  /** `deferred` opened the repository read-only; no index was started. */
  indexing?: "requested" | "deferred";
}

interface OpenFile {
  document: FileDocument;
  draft: string;
}

/**
 * A turn in the conversation.
 *
 * The plan and the context review are turns, not dialogs. They arrive where the
 * request that produced them is and they stay there afterwards, so the
 * transcript is the record of what was approved rather than a list of things
 * that happened next to one.
 */
type Turn =
  | { kind: "human"; id: string; text: string; agentId: string }
  | { kind: "note"; id: string; state: ActivityState; text: string }
  | { kind: "plan"; id: string; taskId: string }
  | { kind: "review"; id: string; taskId: string };

/**
 * Repository trust grants reading only. Everything an agent could do to the
 * machine is a separate grant, so the composer says so before you send.
 */
const AGENTS: ComposerAgent[] = [
  {
    id: "claude",
    name: "Claude",
    permissionScope: "read files in this task's worktree",
    asks: ["write_files", "run_command", "network"],
  },
  {
    id: "codex",
    name: "Codex",
    permissionScope: "read files in this task's worktree",
    asks: ["write_files", "run_command", "network"],
  },
];

/**
 * The scopes a new task can declare. Narrow is the default: a task that turns
 * out to need more can say so, but a task that was never asked cannot.
 */
const SCOPES: ComposerScope[] = [
  {
    id: "touched",
    label: "Files I mention",
    globs: [],
    detail: "Every write stops and asks.",
  },
  { id: "app", label: "apps/**", globs: ["apps/**"] },
  { id: "crates", label: "crates/**", globs: ["crates/**"] },
  { id: "repository", label: "Whole repository", globs: ["**"] },
];

/** The pack budget a run is assembled against. Bytes, as the core measures it. */
const PACK_BUDGET_BYTES = 64 * 1024;

/** How a worktree's lifecycle reads in the shell's state vocabulary. */
export function worktreeActivityState(state: WorktreeState): ActivityState {
  switch (state) {
    case "planned":
      return "queued";
    case "provisioning":
    case "removing":
      return "loading";
    case "ready":
      return "idle";
    case "removed":
      return "done";
    case "orphaned":
      return "stale";
    case "failed":
      return "failed";
    default:
      return "idle";
  }
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

/** The most recent thing the person actually asked for, verbatim. */
function lastRequest(turns: Turn[]): string {
  for (let index = turns.length - 1; index >= 0; index -= 1) {
    const turn = turns[index];
    if (turn.kind === "human") return turn.text;
  }
  return "";
}

/** The first line of a request, as the task's title. */
function titleFrom(text: string) {
  const line = text.trim().split("\n")[0];
  return line.length > 72 ? `${line.slice(0, 69)}…` : line;
}

/**
 * The product composed out of shell regions. No region here supplies its own
 * chrome: the frame owns the rails, headers, panels, states and keymap, and
 * this file only decides what goes in them.
 */
export function ShellWorkspace({
  connection,
  indexing = "requested",
  onOpenProject,
  project,
}: ShellWorkspaceProps) {
  const { dispatch } = useShell();
  const [tasks, setTasks] = useState<WorktreePlan[]>([]);
  const [tasksState, setTasksState] = useState<ActivityState>("loading");
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [reloadToken, setReloadToken] = useState(0);
  const [mentions, setMentions] = useState<MentionTarget[]>([]);
  const [taskSkills, setTaskSkills] = useState<{
    taskId: string;
    items: SkillInstallation[];
  } | null>(null);
  const [files, setFiles] = useState<OpenFile[]>([]);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [revealLine, setRevealLine] = useState<number | null>(null);
  const [fileError, setFileError] = useState<string | null>(null);
  const [agentId, setAgentId] = useState(AGENTS[0].id);
  const [scopeId, setScopeId] = useState(SCOPES[0].id);
  const [draft, setDraft] = useState("");
  const [attachments, setAttachments] = useState<ComposerAttachment[]>([]);
  const [turns, setTurns] = useState<Turn[]>([]);

  const [index, setIndex] = useState<IndexStatus | null>(null);
  const [indexState, setIndexState] = useState<ActivityState>("loading");
  const [startingPoints, setStartingPoints] = useState<StartingPoint[]>([]);
  const [startingPointsState, setStartingPointsState] = useState<ActivityState>("loading");

  const [proposal, setProposal] = useState<WorktreeProposal | null>(null);
  const [planState, setPlanState] = useState<ActivityState>("idle");
  const [planError, setPlanError] = useState<string | null>(null);

  const [pack, setPack] = useState<RunContextPack | null>(null);
  const [packState, setPackState] = useState<ActivityState>("idle");
  const [packError, setPackError] = useState<string | null>(null);
  const [removedRefs, setRemovedRefs] = useState<string[]>([]);

  useEffect(() => {
    let active = true;
    invoke<WorktreePlan[]>("list_tasks", { repositoryId: project.repositoryId })
      .then((items) => {
        if (!active) return;
        setTasks(items);
        setTasksState("idle");
      })
      .catch(() => {
        if (active) setTasksState("failed");
      });
    return () => {
      active = false;
    };
  }, [project.repositoryId, reloadToken]);

  // The state starts as `loading`, so the first read does not have to announce
  // itself; a refresh does, and says so at the point the person asked for one.
  const loadIndex = useCallback(
    () =>
      invoke<IndexStatus>("index_status", { repositoryId: project.repositoryId })
        .then((status) => {
          setIndex(status);
          setIndexState("idle");
        })
        .catch(() => setIndexState("failed")),
    [project.repositoryId],
  );

  useEffect(() => {
    void loadIndex();
  }, [loadIndex]);

  // Re-read when the index moves: what is worth starting depends on what Mars
  // has read. The previous list stays on screen until the new one arrives
  // rather than flashing an empty state at every refresh.
  useEffect(() => {
    let active = true;
    invoke<StartingPoint[]>("starting_points", { repositoryId: project.repositoryId })
      .then((points) => {
        if (!active) return;
        setStartingPoints(points);
        setStartingPointsState("idle");
      })
      .catch(() => {
        if (active) setStartingPointsState("failed");
      });
    return () => {
      active = false;
    };
  }, [project.repositoryId, index]);

  // One page of files is enough to make `@` useful without walking a monorepo.
  useEffect(() => {
    let active = true;
    invoke<FilePage>("list_files", { repositoryId: project.repositoryId, cursor: null, limit: 500 })
      .then((page) => {
        if (!active) return;
        setMentions(
          page.entries.map((entry) => ({
            id: entry.relativePath,
            kind: "file" as const,
            label: entry.relativePath.split("/").at(-1) ?? entry.relativePath,
            detail: entry.relativePath,
          })),
        );
      })
      .catch(() => {
        if (active) setMentions([]);
      });
    return () => {
      active = false;
    };
  }, [project.repositoryId]);

  useEffect(() => {
    if (!selectedTaskId) return;
    let active = true;
    invoke<SkillInstallation[]>("list_task_skills", { taskId: selectedTaskId })
      .then((items) => {
        if (active) setTaskSkills({ taskId: selectedTaskId, items });
      })
      .catch(() => {
        if (active) setTaskSkills({ taskId: selectedTaskId, items: [] });
      });
    return () => {
      active = false;
    };
  }, [selectedTaskId]);

  // Skills belong to the task they were installed for; a stale list is never
  // offered for a different one.
  const skills = taskSkills?.taskId === selectedTaskId ? taskSkills.items : [];

  const selectedTask = useMemo(
    () => tasks.find((task) => task.taskId === selectedTaskId) ?? null,
    [selectedTaskId, tasks],
  );

  const shellTasks = useMemo<ShellTask[]>(
    () =>
      tasks.map((task) => ({
        id: task.taskId,
        title: task.title,
        state: worktreeActivityState(task.state),
        branch: task.branchName,
        detail: task.state,
      })),
    [tasks],
  );

  const openFile = useCallback(
    async (relativePath: string, force = false, line: number | null = null) => {
      setRevealLine(line);
      dispatch({ type: "navigate", destination: "editor" });
      const existing = files.find((file) => file.document.relativePath === relativePath);
      if (existing && !force) {
        setActivePath(relativePath);
        return;
      }
      setFileError(null);
      try {
        const document = await invoke<FileDocument>("read_file", {
          repositoryId: project.repositoryId,
          relativePath,
        });
        setFiles((current) => [
          ...current.filter((file) => file.document.relativePath !== relativePath),
          { document, draft: document.content },
        ]);
        setActivePath(relativePath);
      } catch (caught) {
        setFileError(messageFor(caught));
      }
    },
    [dispatch, files, project.repositoryId],
  );

  const activeFile = files.find((file) => file.document.relativePath === activePath) ?? null;

  function updateActive(update: (file: OpenFile) => OpenFile) {
    if (!activePath) return;
    setFiles((current) =>
      current.map((file) => (file.document.relativePath === activePath ? update(file) : file)),
    );
  }

  function upsertTask(task: WorktreePlan) {
    setTasks((current) => [task, ...current.filter((item) => item.taskId !== task.taskId)]);
    setSelectedTaskId(task.taskId);
  }

  function note(text: string, state: ActivityState = "queued") {
    setTurns((current) => [...current, { kind: "note", id: crypto.randomUUID(), state, text }]);
  }

  /** Assemble the pack for a provisioned task, excluding what was removed. */
  const buildPack = useCallback(
    async (taskId: string, excluded: string[]) => {
      setPackState("loading");
      setPackError(null);
      try {
        const built = await invoke<RunContextPack>("build_run_context_pack", {
          request: {
            taskId,
            agentId,
            adapter: agentId === "codex" ? "codex" : "claude",
            role: "implementer",
            assignment: lastRequest(turns),
            maxBytes: PACK_BUDGET_BYTES,
            excludedEntityRefs: excluded,
          },
        });
        setPack(built);
        setPackState("idle");
      } catch (caught) {
        setPackError(messageFor(caught));
        setPackState("failed");
      }
    },
    [agentId, turns],
  );

  /**
   * A request becomes a plan, not a run. Nothing has touched Git at this point:
   * the plan is assembled from reads, shown as a message, and waits.
   */
  async function submit(text: string) {
    setTurns((current) => [
      ...current,
      { kind: "human", id: crypto.randomUUID(), text, agentId },
    ]);
    setPlanState("loading");
    setPlanError(null);
    const scope = SCOPES.find((candidate) => candidate.id === scopeId) ?? SCOPES[0];
    try {
      const planned = await invoke<WorktreeProposal>("plan_task", {
        request: {
          repositoryId: project.repositoryId,
          title: titleFrom(text),
          intent: text,
          baseRef: index?.branch ?? "HEAD",
          expectedScope: scope.globs,
          setupCommands: [],
        },
      });
      setProposal(planned);
      setPlanState("idle");
      setSelectedTaskId(planned.taskId);
      setReloadToken((token) => token + 1);
      setTurns((current) => [
        ...current,
        { kind: "plan", id: crypto.randomUUID(), taskId: planned.taskId },
      ]);
    } catch (caught) {
      setPlanError(messageFor(caught));
      setPlanState("failed");
      note(`The plan could not be built: ${messageFor(caught)}`, "failed");
    }
  }

  /**
   * One button, two possible steps. Approval is recorded against a fingerprint,
   * so edits must reach the core before they can be approved — and the plan the
   * approval names is the one the core produced from those edits, never the one
   * that was on screen before them.
   */
  async function approvePlan(edits: PlanEdits) {
    if (!proposal) return;
    setPlanState("loading");
    setPlanError(null);
    try {
      let current = proposal;
      if (edits.edited) {
        current = await invoke<WorktreeProposal>("amend_task_plan", {
          taskId: proposal.taskId,
          expectedScope: edits.expectedScope,
          setupCommands: edits.setupCommands satisfies SetupCommandInput[],
        });
        setProposal(current);
      }
      const approved = await invoke<WorktreeProposal>("approve_task_plan", {
        taskId: current.taskId,
        fingerprint: current.fingerprint,
      });
      setProposal(approved);
      const provisioned = await invoke<WorktreeProposal>("provision_approved_task", {
        taskId: approved.taskId,
      });
      setProposal(provisioned);
      setPlanState("idle");
      setReloadToken((token) => token + 1);
      setTurns((current2) => [
        ...current2,
        { kind: "review", id: crypto.randomUUID(), taskId: provisioned.taskId },
      ]);
      await buildPack(provisioned.taskId, []);
    } catch (caught) {
      setPlanError(messageFor(caught));
      setPlanState("failed");
    }
  }

  function toggleEntry(entityRef: string, included: boolean) {
    if (!proposal) return;
    const next = included
      ? removedRefs.filter((ref) => ref !== entityRef)
      : [...removedRefs, entityRef];
    setRemovedRefs(next);
    void buildPack(proposal.taskId, next);
  }

  async function attachFiles(dropped: File[]) {
    for (const file of dropped) {
      try {
        const buffer = new Uint8Array(await file.arrayBuffer());
        const item = await invoke<ContextItem>("capture_context_bytes", {
          repositoryId: project.repositoryId,
          taskId: selectedTaskId,
          title: null,
          originalName: file.name,
          bytes: Array.from(buffer),
          mimeType: file.type || "application/octet-stream",
        });
        setAttachments((current) => [
          ...current,
          {
            id: item.id,
            label: item.originalName ?? item.title,
            // Classification and quarantine are the core's reading of the file.
            // A quarantined attachment says so here rather than looking normal.
            detail: item.quarantineReason ?? item.classification ?? item.mimeType,
          },
        ]);
      } catch (caught) {
        note(`${file.name} could not be attached: ${messageFor(caught)}`, "failed");
      }
    }
  }

  const editorSurface = (
    <div className="flex h-full min-h-0">
      <div className="flex w-56 shrink-0 flex-col border-r border-border">
        <FileExplorer
          activePath={activePath}
          onOpen={(path) => void openFile(path)}
          repositoryId={project.repositoryId}
          repositoryName={project.name}
        />
      </div>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {files.length > 0 ? (
          <>
            <div className="flex h-[var(--shell-header-height)] shrink-0 items-stretch overflow-x-auto border-b border-border">
              {files.map((file) => {
                const current = file.document.relativePath === activePath;
                return (
                  <button
                    key={file.document.relativePath}
                    type="button"
                    aria-current={current ? "page" : undefined}
                    onClick={() => setActivePath(file.document.relativePath)}
                    className={`relative flex items-center gap-1.5 px-2 text-[11px] outline-none before:absolute before:inset-x-1.5 before:bottom-0 before:h-0.5 before:bg-transparent ${
                      current
                        ? "text-foreground before:bg-primary"
                        : "text-muted-foreground hover:text-foreground"
                    }`}
                  >
                    <Mono>{file.document.relativePath.split("/").at(-1)}</Mono>
                    {file.draft !== file.document.content ? (
                      <span aria-label="Unsaved" className="size-1.5 rounded-[1px] bg-primary" />
                    ) : null}
                  </button>
                );
              })}
            </div>
            {activeFile ? (
              <Suspense fallback={<StateBlock state="loading" title="Loading editor" />}>
                <CodeEditor
                  key={activeFile.document.relativePath}
                  document={activeFile.document}
                  onChange={(value) => updateActive((file) => ({ ...file, draft: value }))}
                  onReload={() => void openFile(activeFile.document.relativePath, true)}
                  onSaved={(document) => updateActive(() => ({ document, draft: document.content }))}
                  repositoryId={project.repositoryId}
                  revealLine={revealLine}
                  value={activeFile.draft}
                />
              </Suspense>
            ) : null}
          </>
        ) : (
          <StateBlock
            state={fileError ? "failed" : "idle"}
            title={fileError ? "File could not be opened" : "No file open"}
            description={
              fileError ??
              "Binary, oversized, ignored and out-of-root files are refused by the core, not hidden by the UI."
            }
          />
        )}
      </div>
    </div>
  );

  const transcript = turns.map((turn) => {
    if (turn.kind === "human") {
      return (
        <div key={turn.id} className="flex flex-col gap-1 border-b border-border px-3 py-2.5">
          <MetaLabel>You</MetaLabel>
          <p className="text-[13px] leading-relaxed whitespace-pre-wrap text-foreground">
            {turn.text}
          </p>
        </div>
      );
    }
    if (turn.kind === "note") {
      return (
        <div key={turn.id} className="flex flex-col gap-1 border-b border-border px-3 py-2">
          <StateLabel state={turn.state} />
          <p className="text-[12px] leading-relaxed text-muted-foreground">{turn.text}</p>
        </div>
      );
    }
    if (turn.kind === "plan") {
      return proposal && proposal.taskId === turn.taskId ? (
        <WorktreePlanMessage
          error={planError}
          key={turn.id}
          onApprove={(edits) => void approvePlan(edits)}
          proposal={proposal}
          state={planState}
        />
      ) : null;
    }
    return (
      <div className="border-b border-border" key={turn.id}>
        <RunContextReview
          error={packError}
          onStart={(digest) =>
            note(
              `Context accepted at digest ${digest.slice(0, 12)}. The run surface — timeline, ` +
                `permission prompts and diffs — is the next stage; nothing has been launched.`,
              "queued",
            )
          }
          onToggleEntry={toggleEntry}
          pack={pack}
          removed={removedRefs}
          state={packState}
        />
      </div>
    );
  });

  const home = (
    <RepositoryHome
      index={index}
      indexState={indexing === "deferred" && !index?.counts.files ? "idle" : indexState}
      onIndex={() => {
        setIndexState("loading");
        invoke("index_repository", { repositoryId: project.repositoryId })
          .then(loadIndex)
          .catch(() => setIndexState("failed"));
      }}
      onSelectTask={setSelectedTaskId}
      onUseStartingPoint={setDraft}
      repositoryName={project.name}
      startingPoints={startingPoints}
      startingPointsState={startingPointsState}
      tasks={shellTasks}
    />
  );

  // The conversation is never empty of purpose: with no turns it is the
  // repository home, and with a task selected it is that task's record.
  const conversationBody =
    turns.length > 0 ? (
      <>{transcript}</>
    ) : selectedTask ? (
      <TaskDetail
        key={selectedTask.taskId}
        onOpenWorktree={onOpenProject}
        onTaskChanged={upsertTask}
        task={selectedTask}
      />
    ) : (
      home
    );

  return (
    <ShellFrame
      accountLabel="Local profile"
      taskRail={{
        tasks: shellTasks,
        selectedTaskId,
        onSelectTask: setSelectedTaskId,
        onRetry: () => {
          setTasksState("loading");
          setReloadToken((token) => token + 1);
        },
        state: tasksState,
        headerAction: (
          <CreateTaskDialog onCreated={upsertTask} repositoryId={project.repositoryId} />
        ),
      }}
      workbench={{
        editor: {
          content: editorSurface,
          state: fileError ? "failed" : "idle",
          detail: activePath ?? undefined,
        },
        "source-control": { state: "idle" },
      }}
      conversation={{
        title: selectedTask?.title ?? project.name,
        detail: selectedTask?.branchName,
        state: selectedTask ? worktreeActivityState(selectedTask.state) : "idle",
        children: conversationBody,
        composer: (
          <Composer
            agents={AGENTS}
            attachments={attachments}
            mentions={mentions}
            onDropFiles={(dropped) => void attachFiles(dropped)}
            onRemoveAttachment={(id) =>
              setAttachments((current) => current.filter((item) => item.id !== id))
            }
            onSelectAgent={setAgentId}
            onSelectScope={setScopeId}
            onSubmit={(submission) => void submit(submission.text)}
            onValueChange={setDraft}
            scopes={SCOPES}
            selectedAgentId={agentId}
            selectedScopeId={scopeId}
            skills={skills.map((skill) => ({
              id: skill.id,
              name: skill.name,
              description: `${skill.scope} · ${skill.pinnedHash.slice(0, 12)}`,
              instructions: skill.skillMarkdown,
            }))}
            state={planState === "loading" ? "loading" : "idle"}
            value={draft}
          />
        ),
      }}
      inspector={{
        content: {
          plan:
            proposal && planState !== "loading" ? (
              <WorktreePlanMessage
                error={planError}
                onApprove={(edits) => void approvePlan(edits)}
                proposal={proposal}
                state={planState}
              />
            ) : undefined,
          context: pack ? (
            <RunContextReview
              error={packError}
              onStart={() => undefined}
              onToggleEntry={toggleEntry}
              pack={pack}
              removed={removedRefs}
              state={packState}
            />
          ) : undefined,
          brain: (
            <BrainAnswer
              onOpenSource={(path, line) => void openFile(path, false, line)}
              repositoryId={project.repositoryId}
            />
          ),
        },
        statuses: {
          plan: proposal ? { state: proposal.approvedAt ? "done" : "blocked" } : undefined,
          context: pack ? { state: packState, count: pack.entries.length } : undefined,
        },
      }}
      utility={{
        content: { terminal: <TerminalPanel project={project} /> },
      }}
      status={{
        repository: project.name,
        worktree: selectedTask?.worktreePath,
        branch: selectedTask?.branchName ?? index?.branch ?? "—",
        index: {
          state: indexActivityState(index),
          detail: index ? `${index.counts.files.toLocaleString()} files` : undefined,
        },
        agents: { state: "idle", count: 0 },
        permissions: { state: "idle", pending: 0 },
        connection,
      }}
    />
  );
}
