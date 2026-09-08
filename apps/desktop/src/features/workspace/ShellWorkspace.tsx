import type {
  ActivityEvent,
  ActivityPayload,
  CommandFailure,
  ContextItem,
  FileDocument,
  FilePage,
  IndexStatus,
  InterruptedRun,
  MemoryProposal,
  PendingPermissionRequest,
  PermissionDecisionChoice,
  ProjectSnapshot,
  RecoveryAction,
  RunContextPack,
  SetupCommandInput,
  SkillInstallation,
  StartingPoint,
  TaskProjection,
  WorktreePlan,
  WorktreeProposal,
  WorktreeState,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { lazy, Suspense, useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { ActivityLog } from "@/features/activity/ActivityLog";
import { CompletionSummary } from "@/features/activity/CompletionSummary";
import { InterruptedRunRecovery } from "@/features/activity/InterruptedRunRecovery";
import { RunStatusLine, runActivityState } from "@/features/activity/RunStatusLine";
import { PermissionRequestMessage } from "@/features/agents/PermissionRequestMessage";
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
import { Mono, StateBlock, StateLabel } from "@/shell/primitives";
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
 * Something to say that the activity record does not carry.
 *
 * Human turns, plans and permission requests all live in the log and are
 * rendered from it. A note is what is left: this client's own report of
 * something that failed before it could be recorded, which would otherwise
 * vanish without a trace.
 */
interface Note {
  id: string;
  state: ActivityState;
  text: string;
}

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
  const [notes, setNotes] = useState<Note[]>([]);
  /** The last thing asked for, verbatim, as the pack's assignment. */
  const [assignment, setAssignment] = useState("");

  /**
   * A task's record, tagged with the task it belongs to.
   *
   * Kept together and keyed rather than cleared on selection, so the previous
   * task's log can never be shown for a moment under a new task's heading.
   */
  const [record, setRecord] = useState<{
    taskId: string;
    events: ActivityEvent[];
    projection: TaskProjection;
    pending: PendingPermissionRequest[];
  } | null>(null);
  const [eventsState, setEventsState] = useState<ActivityState>("idle");
  const [decidingId, setDecidingId] = useState<string | null>(null);
  const [permissionError, setPermissionError] = useState<string | null>(null);
  const [interrupted, setInterrupted] = useState<InterruptedRun[]>([]);
  const [recoveryBusy, setRecoveryBusy] = useState(false);
  const [proposals, setProposals] = useState<MemoryProposal[]>([]);
  const [interrupting, setInterrupting] = useState(false);

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

  /**
   * Re-read the task's record.
   *
   * Everything the conversation shows comes from here, so a live run and a
   * reload are the same code path: both ask the log for what they have not seen.
   */
  const loadRecord = useCallback(
    (taskId: string) =>
      Promise.all([
        invoke<ActivityEvent[]>("task_activity", { taskId, afterSequence: 0, limit: 1000 }),
        invoke<TaskProjection>("task_projection", { taskId }),
        invoke<PendingPermissionRequest[]>("pending_permissions", { taskId }),
      ])
        .then(([events, projection, pending]) => {
          setRecord({ taskId, events, projection, pending });
          setEventsState("idle");
        })
        .catch(() => setEventsState("failed")),
    [],
  );

  useEffect(() => {
    if (!selectedTaskId) return;
    void loadRecord(selectedTaskId);
  }, [loadRecord, selectedTaskId, reloadToken]);

  // The agent event is a nudge, not the truth. It says something happened; the
  // record says what, and the record is what gets rendered.
  useEffect(() => {
    if (!selectedTaskId) return;
    const subscription = listen("agent-event", () => {
      void loadRecord(selectedTaskId);
    });
    return () => {
      void subscription.then((unlisten) => unlisten());
    };
  }, [loadRecord, selectedTaskId]);

  // Runs that were alive when Mars stopped. Detection changes nothing and
  // resumes nothing; it only finds what needs a decision.
  useEffect(() => {
    let active = true;
    invoke<InterruptedRun[]>("interrupted_runs")
      .then((runs) => {
        if (active) setInterrupted(runs);
      })
      .catch(() => {
        if (active) setInterrupted([]);
      });
    return () => {
      active = false;
    };
  }, [project.repositoryId]);

  useEffect(() => {
    if (!selectedTaskId) return;
    let active = true;
    invoke<MemoryProposal[]>("memory_pending", {
      filters: { projectId: project.projectId, taskId: selectedTaskId },
    })
      .then((items) => {
        if (active) setProposals(items);
      })
      .catch(() => {
        if (active) setProposals([]);
      });
    return () => {
      active = false;
    };
  }, [project.projectId, selectedTaskId]);

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
    setNotes((current) => [...current, { id: crypto.randomUUID(), state, text }]);
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
            assignment,
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
    [agentId, assignment],
  );

  /**
   * A request becomes a plan, not a run. Nothing has touched Git at this point:
   * the plan is assembled from reads, shown as a message, and waits.
   */
  async function submit(text: string) {
    setAssignment(text);
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
      // The request is recorded against the task it produced, so the log reads
      // in the order things actually happened.
      await invoke("record_message", { taskId: planned.taskId, text });
      setProposal(planned);
      setPlanState("idle");
      setSelectedTaskId(planned.taskId);
      setReloadToken((token) => token + 1);
      await loadRecord(planned.taskId);
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
      await loadRecord(provisioned.taskId);
      await buildPack(provisioned.taskId, []);
    } catch (caught) {
      setPlanError(messageFor(caught));
      setPlanState("failed");
    }
  }

  /** Answer a permission request and let the agent waiting on it carry on. */
  async function decide(requestId: string, choice: PermissionDecisionChoice) {
    setDecidingId(requestId);
    setPermissionError(null);
    try {
      await invoke("decide_permission", { requestId, decision: choice });
      if (selectedTaskId) await loadRecord(selectedTaskId);
    } catch (caught) {
      setPermissionError(messageFor(caught));
    } finally {
      setDecidingId(null);
    }
  }

  /**
   * Record which way out of an interrupted run was chosen. The choice reaches
   * the transcript first; nothing is resumed here, and nothing was resumed
   * before it was asked for.
   */
  async function recover(run: InterruptedRun, action: RecoveryAction) {
    setRecoveryBusy(true);
    try {
      await invoke("resolve_interrupted_run", { runId: run.runId, action });
      setInterrupted((current) =>
        current.filter((candidate) => candidate.runId !== run.runId),
      );
      if (action !== "discard") {
        note(
          `Recorded the choice to ${action} run ${run.runId.slice(0, 12)}. Carrying it out is ` +
            `the run surface's job, and it has not run.`,
          "queued",
        );
      }
      await loadRecord(run.taskId);
    } catch (caught) {
      note(`The recovery choice could not be recorded: ${messageFor(caught)}`, "failed");
    } finally {
      setRecoveryBusy(false);
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

  // A record belonging to a different task is not this task's record.
  const current = record?.taskId === selectedTaskId ? record : null;
  const events = current?.events ?? [];
  const projection = current?.projection ?? null;
  const pending = current?.pending ?? [];

  const activeRun = projection?.runs.at(-1) ?? null;
  const latestEvent = events.at(-1) ?? null;
  const pendingIds = new Set(pending.map((request) => request.id));

  const loadPayload = useCallback(
    (digest: string) => invoke<ActivityPayload>("activity_payload", { digest }),
    [],
  );

  /**
   * Where a card belongs in the record.
   *
   * Each of these replaces the plain row for the event that produced it, so the
   * plan sits where it was planned and the permission request sits where the
   * agent stopped — rather than in a tray beside the transcript.
   */
  function renderEvent(event: ActivityEvent) {
    if (event.kind === "worktree_planned" && proposal && proposal.taskId === event.taskId) {
      return (
        <WorktreePlanMessage
          error={planError}
          onApprove={(edits) => void approvePlan(edits)}
          proposal={proposal}
          state={planState}
        />
      );
    }

    if (event.kind === "permission_request") {
      const requestId = (event.payload as { requestId?: string } | null)?.requestId;
      const request = pending.find((candidate) => candidate.id === requestId);
      // Only a request still waiting becomes a question. One already answered
      // stays in the record as the plain row it is, next to its decision.
      if (request && pendingIds.has(request.id)) {
        return (
          <PermissionRequestMessage
            busy={decidingId === request.id}
            error={decidingId === request.id ? permissionError : null}
            onDecide={(choice) => void decide(request.id, choice)}
            request={request}
          />
        );
      }
    }

    if (event.kind === "done" && projection) {
      return (
        <CompletionSummary
          branchName={projection.worktree?.branchName ?? selectedTask?.branchName}
          onPublish={() => dispatch({ type: "navigate", destination: "source-control" })}
          onReviewDiff={() => dispatch({ type: "navigate", destination: "source-control" })}
          onReviewMemory={() => dispatch({ type: "openInspector", tab: "brain" })}
          onRunChecks={() => dispatch({ type: "openUtility", tab: "checks" })}
          projection={projection}
          proposals={proposals}
        />
      );
    }

    return null;
  }

  const recovery = interrupted
    .filter((run) => run.taskId === selectedTaskId)
    .map((run) => (
      <InterruptedRunRecovery
        busy={recoveryBusy}
        completed={[
          ...(projection?.commands ?? [])
            .filter((command) => command.runId === run.runId && command.status === "completed")
            .map((command) => ({ label: command.command, detail: `exit ${command.exitCode ?? 0}` })),
          ...(projection?.tests ?? []).map((test) => ({
            label: test.command,
            detail: test.status,
          })),
        ]}
        key={run.runId}
        onChoose={(action) => void recover(run, action)}
        run={run}
      />
    ));

  const noteRows = notes.map((entry) => (
    <div className="flex flex-col gap-1 border-b border-border px-3 py-2" key={entry.id}>
      <StateLabel state={entry.state} />
      <p className="text-[12px] leading-relaxed text-muted-foreground">{entry.text}</p>
    </div>
  ));

  const transcript = (
    <>
      {recovery}
      <ActivityLog
        events={events}
        loadPayload={loadPayload}
        onOpenFile={(path, line) => void openFile(path, false, line)}
        renderEvent={renderEvent}
      />
      {pack ? (
        <div className="border-b border-border">
          <RunContextReview
            error={packError}
            onStart={(digest) =>
              note(
                `Context accepted at digest ${digest.slice(0, 12)}. The run surface is wired to ` +
                  `the activity record; launching an agent into it is the next stage.`,
                "queued",
              )
            }
            onToggleEntry={toggleEntry}
            pack={pack}
            removed={removedRefs}
            state={packState}
          />
        </div>
      ) : null}
      {noteRows}
    </>
  );

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

  // The conversation is never empty of purpose: with nothing recorded it is the
  // repository home, and with a record it is that record.
  const hasRecord = events.length > 0 || notes.length > 0 || recovery.length > 0;
  const conversationBody = hasRecord ? (
    transcript
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
          <>
            <RunStatusLine
              blockedSummary={pending[0]?.summary}
              interrupting={interrupting}
              latest={latestEvent}
              onInterrupt={() => {
                setInterrupting(true);
                invoke("cancel_agent", { sessionId: activeRun?.runId })
                  .catch((caught: unknown) =>
                    note(`The run could not be interrupted: ${messageFor(caught)}`, "failed"),
                  )
                  .finally(() => setInterrupting(false));
              }}
              run={activeRun}
            />
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
            state={
              activeRun?.state === "blocked"
                ? "blocked"
                : activeRun?.state === "running"
                  ? "streaming"
                  : planState === "loading"
                    ? "loading"
                    : "idle"
            }
            blockedReason={pending[0]?.summary}
            value={draft}
            />
          </>
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
          diff: projection ? (
            <ul className="flex flex-col">
              {projection.filesTouched.map((file) => (
                <li key={file.path}>
                  <button
                    type="button"
                    onClick={() => void openFile(file.path)}
                    className="flex w-full items-baseline gap-2 border-b border-border px-2 py-1 text-left outline-none hover:bg-surface-raised focus-visible:bg-surface-raised"
                  >
                    <Mono className="min-w-0 flex-1 truncate text-foreground">{file.path}</Mono>
                    <Mono className="shrink-0 text-faint">{file.change}</Mono>
                  </button>
                </li>
              ))}
            </ul>
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
          diff: projection
            ? { state: eventsState, count: projection.filesTouched.length }
            : undefined,
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
        agents: {
          state: activeRun ? runActivityState(activeRun) : "idle",
          count: activeRun && activeRun.state === "running" ? 1 : 0,
        },
        permissions: {
          state: pending.length > 0 ? "blocked" : "idle",
          pending: pending.length,
        },
        connection,
      }}
    />
  );
}
