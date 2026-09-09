import type {
  ActivityEvent,
  ActivityPayload,
  ActivityWindow,
  BaseDrift,
  DriftChoice,
  BrainAnswer as BrainAnswerValue,
  BrainStreamPayload,
  Citation,
  Claim,
  CommandFailure,
  ContextItem,
  FileDocument,
  FilePage,
  GrantView,
  AdapterProbe,
  IndexStatus,
  InterruptedRun,
  Memory,
  MemoryFilters,
  MemoryProposal,
  FindingStatus,
  PendingPermissionRequest,
  PermissionDecisionChoice,
  PlannedCheck,
  PullRequestPlan,
  PullRequestSnapshot,
  ProjectSnapshot,
  RecoveryAction,
  ReviewFinding,
  ReviewHandoff,
  ReviewRecord,
  RunContextPack,
  SetupCommandInput,
  SkillInstallation,
  PaletteHit,
  SearchResults,
  StartingPoint,
  TaskCommit,
  TaskProjection,
  WorktreePlan,
  WorktreeProposal,
  WorktreeState,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { MaximizeIcon } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { ActivityLog } from "@/features/activity/ActivityLog";
import { CompletionSummary } from "@/features/activity/CompletionSummary";
import { InterruptedRunRecovery } from "@/features/activity/InterruptedRunRecovery";
import { RunStatusLine, runActivityState } from "@/features/activity/RunStatusLine";
import { BrainAnswerMessage } from "@/features/brain/BrainAnswerMessage";
import { BrainExplorer } from "@/features/brain/BrainExplorer";
import { LibraryOverlay, type LibraryTab } from "@/features/library/LibraryOverlay";
import { MemoryBrowser, type MemoryFilterState } from "@/features/memory/MemoryBrowser";
import { CommandPalette } from "@/features/palette/CommandPalette";
import { PullRequestActivity } from "@/features/forge/PullRequestActivity";
import { Settings } from "@/features/settings/Settings";
import { SkillsLibrary } from "@/features/skills/SkillsLibrary";
import { SourceControl } from "@/features/review/SourceControl";
import { BaseDriftMessage } from "@/features/tasks/BaseDriftMessage";
import {
  ChecksPanel,
  ProblemsPanel,
  TestsPanel,
  problemsFromProjection,
} from "@/features/utility/UtilityPanels";
import { FocusMode } from "@/features/focus/FocusMode";
import { PublishPreview } from "@/features/forge/PublishPreview";
import { ApprovalGate } from "@/features/review/ApprovalGate";
import { DiffReview } from "@/features/review/DiffReview";
import { ReviewFindings } from "@/features/review/ReviewFindings";
import { parseUnifiedDiff, untrackedFiles, type DiffFile } from "@/features/review/unified-diff";
import { PermissionRequestMessage } from "@/features/agents/PermissionRequestMessage";
import { RunContextReview } from "@/features/agents/RunContextReview";
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
import { anchorPaths, type BlobAnchor } from "@/shell/staleness";
import { useWorkingTreeShas } from "@/lib/working-tree";
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

/**
 * How much of a task's record the conversation holds at once.
 *
 * The record is authoritative and unbounded; this is only what is rendered.
 * The page is taken from the *end* of the log, because that is where a reader
 * arrives, and what sits before it is counted rather than dropped silently.
 */
const RECORD_PAGE = 400;

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
  /**
   * Bumped whenever the working tree may have moved under what is already on
   * screen: a save from the editor, a fresh read of the task's diff.
   *
   * Not bumped per agent event. A run emits hundreds, and re-checking every
   * citation on each one would cost more than the warning is worth; the diff
   * read that follows the run is close enough in time and cheap enough to be
   * the trigger instead.
   */
  const [treeRevision, setTreeRevision] = useState(0);
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
    /** Events before the first one held. Named, so the screen can say so. */
    earlier: number;
    projection: TaskProjection;
    pending: PendingPermissionRequest[];
  } | null>(null);
  /**
   * How much of the record the conversation is holding.
   *
   * Grown by the reader asking for more, and never shrunk while they are
   * reading: a re-read that dropped back to one page would pull the earlier
   * turns they just asked for out from under them.
   */
  const [recordLimit, setRecordLimit] = useState(RECORD_PAGE);
  // Every re-read — a permission answered, a save, an agent event — must use
  // the window the reader is actually on, or answering a prompt would silently
  // drop the earlier turns they had just asked for.
  const recordLimitRef = useRef(RECORD_PAGE);
  recordLimitRef.current = recordLimit;
  const [eventsState, setEventsState] = useState<ActivityState>("idle");
  const [decidingId, setDecidingId] = useState<string | null>(null);
  const [permissionError, setPermissionError] = useState<string | null>(null);
  const [interrupted, setInterrupted] = useState<InterruptedRun[]>([]);
  const [recoveryBusy, setRecoveryBusy] = useState(false);
  const [proposals, setProposals] = useState<MemoryProposal[]>([]);
  const [interrupting, setInterrupting] = useState(false);

  const [diff, setDiff] = useState<{
    taskId: string;
    files: DiffFile[];
    headOid: string | null;
    /** The version of the change itself, for anything anchored to it. */
    diffSha256: string;
    branchName: string;
  } | null>(null);
  const [diffState, setDiffState] = useState<ActivityState>("idle");
  const [diffError, setDiffError] = useState<string | null>(null);

  const [review, setReview] = useState<ReviewRecord | null>(null);
  const [reviewState, setReviewState] = useState<ActivityState>("idle");
  const [busyFindingId, setBusyFindingId] = useState<string | null>(null);

  const [library, setLibrary] = useState<LibraryTab | null>(null);
  const [agentProbes, setAgentProbes] = useState<
    { id: string; name: string; probe: AdapterProbe | null; error: string | null }[]
  >([]);
  const [probeState, setProbeState] = useState<ActivityState>("idle");

  const [paletteOpen, setPaletteOpen] = useState(false);
  const [paletteHits, setPaletteHits] = useState<PaletteHit[]>([]);
  const [paletteState, setPaletteState] = useState<ActivityState>("idle");

  const [memories, setMemories] = useState<Memory[]>([]);
  const [memoryState, setMemoryState] = useState<ActivityState>("idle");
  const [memoryFilters, setMemoryFilters] = useState<MemoryFilterState>({
    scopes: [],
    kinds: [],
    statuses: [],
    contains: "",
  });
  const [busyMemoryId, setBusyMemoryId] = useState<string | null>(null);

  const [grants, setGrants] = useState<GrantView[]>([]);
  const [grantsState, setGrantsState] = useState<ActivityState>("idle");
  const [busyGrantId, setBusyGrantId] = useState<string | null>(null);

  const [drift, setDrift] = useState<BaseDrift | null>(null);
  const [driftBusy, setDriftBusy] = useState(false);

  const [pullRequest, setPullRequest] = useState<PullRequestSnapshot | null>(null);
  const [pullRequestState, setPullRequestState] = useState<ActivityState>("idle");

  const [searchResults, setSearchResults] = useState<SearchResults | null>(null);
  const [searchState, setSearchState] = useState<ActivityState>("idle");

  const [focus, setFocus] = useState(false);
  /** Log length when focus mode was entered, so "new" means new to the reader. */
  const [focusFrom, setFocusFrom] = useState(0);
  const [focusTerminal, setFocusTerminal] = useState(false);

  const [publish, setPublish] = useState<{
    plan: PullRequestPlan | null;
    commits: TaskCommit[];
    checks: PlannedCheck[];
    state: ActivityState;
    error: string | null;
    busy: boolean;
  } | null>(null);

  /**
   * A brain question and its answer, as a turn in the conversation.
   *
   * `streamed` is what the pipeline has said so far. It is kept beside the
   * answer rather than merged into it: the core prunes citations that fail
   * validation before it returns, so the stream is a preview and the resolved
   * answer replaces it wholesale.
   */
  const [ask, setAsk] = useState<{
    id: string;
    question: string;
    answer: BrainAnswerValue | null;
    streamed: Claim[];
    streamedCitations: Citation[];
    stage: string | null;
    state: ActivityState;
    error: string | null;
  } | null>(null);

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
    (taskId: string, limit: number = recordLimitRef.current) =>
      Promise.all([
        // From the end of the log, not the beginning. Reading forward would pin
        // a long task to its first page and never show the live tail — the
        // transcript would appear to stop while the run was still going.
        invoke<ActivityWindow>("task_activity_window", {
          taskId,
          beforeSequence: null,
          limit,
        }),
        invoke<TaskProjection>("task_projection", { taskId }),
        invoke<PendingPermissionRequest[]>("pending_permissions", { taskId }),
      ])
        .then(([window, projection, pending]) => {
          setRecord({
            taskId,
            events: window.events,
            earlier: window.earlier,
            projection,
            pending,
          });
          setEventsState("idle");
          // A run that has just started has output to show and somewhere to
          // show it: the utility panel earns the space, and the inspector can
          // wait as a rail. Refused outright for any region already moved by
          // hand, so this can never take a panel from under a reader.
          const run = projection.runs.at(-1);
          dispatch({
            type: "suggestLayout",
            inspectorCollapsed: run?.state === "running",
            utilityOpen: run?.state === "running",
            utilityTab: run?.state === "running" ? "tests" : undefined,
          });
        })
        .catch(() => setEventsState("failed")),
    [dispatch],
  );

  useEffect(() => {
    if (!selectedTaskId) return;
    void loadRecord(selectedTaskId, recordLimit);
  }, [loadRecord, recordLimit, selectedTaskId, reloadToken]);

  // A different task starts at one page again. The limit belongs to the reading
  // of one record, not to the session.
  useEffect(() => {
    setRecordLimit(RECORD_PAGE);
  }, [selectedTaskId]);

  /**
   * Project Brain's pipeline, as it runs.
   *
   * Mounted once for the life of the workspace rather than per question, so a
   * frame that arrives in the gap between asking and the listener attaching is
   * not lost. Frames name the stream they belong to and anything else is
   * dropped — a late answer must never land in the turn that replaced it.
   */
  useEffect(() => {
    const subscription = listen<BrainStreamPayload>("brain-event", ({ payload }) => {
      setAsk((current) => {
        if (!current || current.id !== payload.streamId) return current;
        // Once the call has resolved the answer is authoritative; trailing
        // frames cannot add to it.
        if (current.answer) return current;
        const event = payload.event;
        switch (event.type) {
          case "planned":
            return { ...current, stage: `rewrote into ${event.queries.length} queries` };
          case "retrieved":
            return {
              ...current,
              stage:
                event.stalePaths > 0
                  ? `read ${event.candidates} candidates · ${event.stalePaths} from changed files`
                  : `read ${event.candidates} candidates`,
            };
          case "packed":
            return {
              ...current,
              stage: `packed ${event.sources} sources${event.dropped > 0 ? `, dropped ${event.dropped}` : ""}`,
            };
          case "synthesized":
            return { ...current, stage: `checking ${event.claims} claims against the tree` };
          case "citation":
            return {
              ...current,
              state: "streaming",
              streamedCitations: [...current.streamedCitations, event],
            };
          case "claim":
            return { ...current, state: "streaming", streamed: [...current.streamed, event] };
          case "cancelled":
            return { ...current, state: "failed", error: `cancelled at ${event.stage}` };
          default:
            return current;
        }
      });
    });
    return () => {
      void subscription.then((unlisten) => unlisten());
    };
  }, []);

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

  /**
   * The files the brain answer on screen rests on, and what they hold now.
   *
   * An answer stays in the conversation after the repository has moved on, so
   * this is re-read whenever the index changes or the workspace reloads. The
   * answer itself is never rewritten — it said what it said — but it stops
   * claiming its line numbers are current.
   */
  const askCitations = useMemo<BlobAnchor[]>(
    () =>
      (ask?.answer?.citations ?? ask?.streamedCitations ?? []).map((citation) => ({
        path: citation.path,
        sha: citation.blobSha,
      })),
    [ask?.answer?.citations, ask?.streamedCitations],
  );
  const askShas = useWorkingTreeShas(
    project.repositoryId,
    anchorPaths(askCitations),
    `${index?.headOid ?? ""}:${index?.lastRun?.runId ?? ""}:${treeRevision}:${reloadToken}`,
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

  /**
   * Read the change in the task worktree.
   *
   * Untracked files are listed alongside the diff because a diff against HEAD
   * cannot mention a file Git has never seen, and a review that silently
   * omitted every new file would be worse than no review.
   */
  const loadDiff = useCallback(
    (taskId: string, branch: string) =>
      invoke<{
        diff: string;
        untracked: string[];
        headOid: string | null;
        diffSha256: string;
        branchName: string;
      }>("task_diff", { taskId })
        .then((result) => {
          setDiff({
            taskId,
            files: [...parseUnifiedDiff(result.diff), ...untrackedFiles(result.untracked)],
            headOid: result.headOid,
            diffSha256: result.diffSha256,
            branchName: result.branchName || branch,
          });
          setDiffState("idle");
          setDiffError(null);
          // The worktree was just read. Anything on screen anchored to a blob
          // sha can be re-checked against what it holds now.
          setTreeRevision((revision) => revision + 1);
        })
        .catch((caught: unknown) => {
          setDiffError(messageFor(caught));
          setDiffState("failed");
        }),
    [],
  );

  useEffect(() => {
    if (!selectedTaskId || !selectedTask || selectedTask.state !== "ready") return;
    void loadDiff(selectedTaskId, selectedTask.branchName);
  }, [loadDiff, selectedTask, selectedTaskId, reloadToken]);

  useEffect(() => {
    if (!selectedTaskId) return;
    let active = true;
    invoke<ReviewRecord[]>("list_task_reviews", { taskId: selectedTaskId })
      .then((records) => {
        if (!active) return;
        setReview(records.at(-1) ?? null);
        setReviewState("idle");
      })
      .catch(() => {
        if (!active) return;
        setReview(null);
        setReviewState("failed");
      });
    return () => {
      active = false;
    };
  }, [selectedTaskId, reloadToken]);

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
    // `/ask` is a question about the repository, not work to be done. It needs
    // no branch and no worktree, so it never becomes a task.
    const asked = /^\/ask\s+/.exec(text);
    if (asked) {
      await askBrain(text.slice(asked[0].length));
      return;
    }

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

  /**
   * A line comment is a message about the task, so it goes where every other
   * message about the task goes. Nothing is stored beside the diff.
   */
  async function comment(path: string, line: number, text: string, side: "old" | "new") {
    if (!selectedTaskId) return;
    try {
      await invoke("record_message", {
        taskId: selectedTaskId,
        text: `${path}:${line}${side === "old" ? " (before the change)" : ""} — ${text}`,
      });
      await loadRecord(selectedTaskId);
    } catch (caught) {
      note(`The comment could not be recorded: ${messageFor(caught)}`, "failed");
    }
  }

  /**
   * Hand the change to the other agent.
   *
   * The handoff is built first and shown to have a digest before anything runs,
   * so what the reviewer was given is recorded rather than reconstructed later.
   */
  async function runReview() {
    if (!selectedTaskId) return;
    setReviewState("loading");
    try {
      const handoff = await invoke<ReviewHandoff>("build_review_handoff", {
        taskId: selectedTaskId,
        reviewerAdapter: agentId === "codex" ? "claude" : "codex",
        maxBytes: PACK_BUDGET_BYTES,
      });
      const record = await invoke<ReviewRecord>("run_cross_agent_review", {
        handoff,
        modelPermission: true,
      });
      setReview(record);
      setReviewState("idle");
      await loadRecord(selectedTaskId);
    } catch (caught) {
      setReviewState("failed");
      note(`The review could not be run: ${messageFor(caught)}`, "failed");
    }
  }

  async function decideFinding(finding: ReviewFinding, status: FindingStatus) {
    setBusyFindingId(finding.id);
    try {
      await invoke("set_review_finding_status", { findingId: finding.id, status });
      setReloadToken((token) => token + 1);
    } catch (caught) {
      note(`The finding could not be updated: ${messageFor(caught)}`, "failed");
    } finally {
      setBusyFindingId(null);
    }
  }

  /**
   * Approval is a message. Who released this change and what they said about it
   * belongs in the same record as everything else that happened to the task.
   */
  async function approve(noteText: string) {
    if (!selectedTaskId) return;
    const outstanding = (review?.findings ?? []).filter(
      (finding) => finding.status === "open" || finding.status === "sent",
    );
    try {
      await invoke("record_message", {
        taskId: selectedTaskId,
        text:
          `Approved this change for release.` +
          (noteText ? `\n\n${noteText}` : "") +
          (outstanding.length > 0
            ? `\n\nApproved with ${outstanding.length} finding(s) still open.`
            : ""),
      });
      await loadRecord(selectedTaskId);
    } catch (caught) {
      note(`The approval could not be recorded: ${messageFor(caught)}`, "failed");
    }
  }

  /**
   * Build the publish plan. Reads the remote and the worktree; pushes nothing.
   */
  const planPublish = useCallback(
    async (
      taskId: string,
      next?: { title: string; body: string; draft: boolean; reviewers: string[] },
    ) => {
      setPublish((current) => ({
        plan: current?.plan ?? null,
        commits: current?.commits ?? [],
        checks: current?.checks ?? [],
        state: "loading",
        error: null,
        busy: false,
      }));
      const title = next?.title ?? selectedTask?.title ?? "";
      try {
        const [plan, commits, checks] = await Promise.all([
          invoke<PullRequestPlan>("preview_pull_request", {
            taskId,
            remoteName: "origin",
            title,
            body: next?.body ?? selectedTask?.intent ?? "",
            draft: next?.draft ?? false,
            reviewers: next?.reviewers ?? [],
          }),
          invoke<TaskCommit[]>("task_commits", { taskId }).catch(() => [] as TaskCommit[]),
          invoke<PlannedCheck[]>("planned_checks", { taskId }).catch(() => [] as PlannedCheck[]),
        ]);
        setPublish({ plan, commits, checks, state: "idle", error: null, busy: false });
      } catch (caught) {
        setPublish({
          plan: null,
          commits: [],
          checks: [],
          state: "failed",
          error: messageFor(caught),
          busy: false,
        });
      }
    },
    [selectedTask],
  );

  /**
   * Push and open the pull request.
   *
   * Both approvals are given here because the person pressed the one button
   * that says what it does. The core still re-plans and refuses if anything
   * moved since the preview, so this cannot release something unreviewed.
   */
  async function doPublish(plan: PullRequestPlan) {
    setPublish((current) => (current ? { ...current, busy: true, error: null } : current));
    try {
      const snapshot = await invoke<{ number: number; url: string }>(
        "create_github_pull_request",
        { plan, pushApproved: true, forgeApproved: true },
      );
      if (selectedTaskId) {
        await invoke("record_message", {
          taskId: selectedTaskId,
          text: `Pushed ${plan.head} and opened pull request #${snapshot.number}.\n${snapshot.url}`,
        });
        await loadRecord(selectedTaskId);
      }
      setPublish(null);
    } catch (caught) {
      setPublish((current) =>
        current ? { ...current, busy: false, error: messageFor(caught) } : current,
      );
    }
  }

  const loadMemories = useCallback(
    (filters: MemoryFilterState) => {
      setMemoryState("loading");
      return invoke<Memory[]>("memory_list", {
        filters: {
          projectId: project.projectId,
          repositoryId: project.repositoryId,
          scopes: filters.scopes,
          kinds: filters.kinds,
          statuses: filters.statuses,
          contains: filters.contains || null,
          limit: 200,
        } satisfies MemoryFilters,
      })
        .then((items) => {
          setMemories(items);
          setMemoryState("idle");
        })
        .catch(() => setMemoryState("failed"));
    },
    [project.projectId, project.repositoryId],
  );

  useEffect(() => {
    void loadMemories(memoryFilters);
  }, [loadMemories, memoryFilters]);

  useEffect(() => {
    if (library !== "settings" || agentProbes.length > 0) return;
    setProbeState("loading");
    void Promise.all(
      AGENTS.map((agent) =>
        invoke<AdapterProbe>("probe_agent", { adapter: agent.id })
          .then((probe) => ({ id: agent.id, name: agent.name, probe, error: null }))
          .catch((caught: unknown) => ({
            id: agent.id,
            name: agent.name,
            probe: null,
            error: messageFor(caught),
          })),
      ),
    ).then((probes) => {
      setAgentProbes(probes);
      setProbeState("idle");
    });
  }, [agentProbes.length, library]);

  const loadGrants = useCallback(() => {
    setGrantsState("loading");
    return invoke<GrantView[]>("permission_grants", { taskId: null })
      .then((items) => {
        setGrants(items);
        setGrantsState("idle");
      })
      .catch(() => setGrantsState("failed"));
  }, []);

  useEffect(() => {
    void loadGrants();
  }, [loadGrants]);

  // Drift is read per task: the base can only move under a plan that pinned it.
  useEffect(() => {
    if (!selectedTaskId) return;
    let active = true;
    invoke<BaseDrift | null>("task_base_drift", { taskId: selectedTaskId })
      .then((found) => {
        if (active) setDrift(found);
      })
      .catch(() => {
        if (active) setDrift(null);
      });
    return () => {
      active = false;
    };
  }, [selectedTaskId, reloadToken]);

  useEffect(() => {
    if (!selectedTaskId) return;
    let active = true;
    invoke<PullRequestSnapshot | null>("get_task_pull_request", { taskId: selectedTaskId })
      .then((snapshot) => {
        if (active) setPullRequest(snapshot);
      })
      .catch(() => {
        if (active) setPullRequest(null);
      });
    return () => {
      active = false;
    };
  }, [selectedTaskId, reloadToken]);

  /** Ask the project brain, in the conversation, and answer there too. */
  /**
   * Ask the repository a question, and show the answer being built.
   *
   * The stream id is the question's own id, so a second question asked before
   * the first returns cannot pour its claims into the first one's turn. Frames
   * for any other id are dropped rather than queued: the turn they belonged to
   * is already gone.
   */
  async function askBrain(question: string) {
    const id = crypto.randomUUID();
    setAsk({
      id,
      question,
      answer: null,
      streamed: [],
      streamedCitations: [],
      stage: null,
      state: "loading",
      error: null,
    });
    try {
      const answer = await invoke<BrainAnswerValue>("brain_ask", {
        repositoryId: project.repositoryId,
        question,
        budgetTokens: null,
        maxSources: null,
        streamId: id,
      });
      setAsk((current) =>
        current?.id === id
          ? { ...current, answer, stage: null, state: "idle", error: null }
          : current,
      );
    } catch (caught) {
      setAsk((current) =>
        current?.id === id
          ? { ...current, answer: null, stage: null, state: "failed", error: messageFor(caught) }
          : current,
      );
    }
  }

  async function revokeGrant(grant: GrantView) {
    setBusyGrantId(grant.id);
    try {
      await invoke("revoke_permission_grant", { grantId: grant.id });
      await loadGrants();
    } catch (caught) {
      note(`The grant could not be revoked: ${messageFor(caught)}`, "failed");
    } finally {
      setBusyGrantId(null);
    }
  }

  async function memoryAction(id: string, run: () => Promise<unknown>) {
    setBusyMemoryId(id);
    try {
      await run();
      await loadMemories(memoryFilters);
      const items = await invoke<MemoryProposal[]>("memory_pending", {
        filters: { projectId: project.projectId },
      }).catch(() => [] as MemoryProposal[]);
      setProposals(items);
    } catch (caught) {
      note(`That memory could not be updated: ${messageFor(caught)}`, "failed");
    } finally {
      setBusyMemoryId(null);
    }
  }

  async function resolveDrift(choice: DriftChoice) {
    if (!selectedTaskId) return;
    setDriftBusy(true);
    try {
      await invoke("resolve_task_base_drift", { taskId: selectedTaskId, choice });
      setDrift(null);
      setReloadToken((token) => token + 1);
      await loadRecord(selectedTaskId);
    } catch (caught) {
      note(`The base drift could not be resolved: ${messageFor(caught)}`, "failed");
    } finally {
      setDriftBusy(false);
    }
  }

  const runSearch = useCallback(
    (query: string) => {
      setSearchState("loading");
      invoke<SearchResults>("search_code", {
        repositoryId: project.repositoryId,
        query,
        limit: 20,
      })
        .then((found) => {
          setSearchResults(found);
          setSearchState("idle");
        })
        .catch(() => setSearchState("failed"));
    },
    [project.repositoryId],
  );

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "p") {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  function searchPalette(query: string) {
    if (query.trim().length === 0) {
      setPaletteHits([]);
      return;
    }
    setPaletteState("loading");
    invoke<PaletteHit[]>("search_everything", {
      repositoryId: project.repositoryId,
      projectId: project.projectId,
      query,
    })
      .then((hits) => {
        setPaletteHits(hits);
        setPaletteState("idle");
      })
      .catch(() => setPaletteState("failed"));
  }

  /** A palette hit is a place, and each kind is reached its own way. */
  function jumpTo(hit: PaletteHit) {
    setPaletteOpen(false);
    if (hit.kind === "task" || hit.kind === "message") {
      if (hit.taskId) setSelectedTaskId(hit.taskId);
      dispatch({ type: "navigate", destination: "conversation" });
      return;
    }
    if (hit.kind === "file" || hit.kind === "symbol") {
      void openFile(hit.target, false, hit.line);
      return;
    }
    dispatch({ type: "openInspector", tab: "brain" });
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
            <div className="flex h-[var(--shell-header-height)] shrink-0 items-stretch border-b border-border">
              <div className="flex min-w-0 flex-1 items-stretch overflow-x-auto">
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
              <button
                type="button"
                onClick={() => {
                  setFocusFrom(events.length);
                  setFocus(true);
                }}
                className="flex shrink-0 items-center gap-1 border-l border-border px-2 text-[11px] text-muted-foreground outline-none hover:text-foreground focus-visible:text-foreground"
              >
                <MaximizeIcon className="size-3" />
                Focus
              </button>
            </div>
            {activeFile ? (
              <Suspense fallback={<StateBlock state="loading" title="Loading editor" />}>
                <CodeEditor
                  key={activeFile.document.relativePath}
                  document={activeFile.document}
                  onChange={(value) => updateActive((file) => ({ ...file, draft: value }))}
                  onReload={() => void openFile(activeFile.document.relativePath, true)}
                  onSaved={(document) => {
                    updateActive(() => ({ document, draft: document.content }));
                    setTreeRevision((revision) => revision + 1);
                  }}
                  repositoryId={project.repositoryId}
                  revealLine={revealLine}
                  taskId={selectedTaskId}
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
  const earlier = current?.earlier ?? 0;
  const projection = current?.projection ?? null;
  const pending = current?.pending ?? [];

  const problems = useMemo(() => problemsFromProjection(projection), [projection]);

  const activeRun = projection?.runs.at(-1) ?? null;

  /**
   * Why the diff on screen may already be behind the worktree.
   *
   * The panel is not re-read while a run writes — that would move the reader's
   * place out from under them mid-hunk. It says so instead, and offers the
   * re-read as their move.
   */
  const diffStaleReason =
    activeRun?.state === "running"
      ? "an agent is still writing here; this is the worktree as it was last read"
      : null;
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
          onPublish={() => {
            if (selectedTaskId) void planPublish(selectedTaskId);
          }}
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
      {/* What the conversation is not showing, said rather than left out. The
          record is authoritative and unbounded; this column holds a window on
          the end of it, and a reader must be able to see the edge of that
          window and step past it. */}
      {earlier > 0 ? (
        <div className="flex items-center gap-2 border-b border-border px-3 py-1.5">
          <StateLabel state="stale" detail={`${earlier} earlier event(s) not shown`} />
          <button
            type="button"
            onClick={() => setRecordLimit((limit) => limit + RECORD_PAGE)}
            className="ml-auto shrink-0 border border-border px-1.5 py-0.5 text-[11px] text-foreground outline-none hover:border-border-strong"
          >
            Show {Math.min(earlier, RECORD_PAGE)} more
          </button>
        </div>
      ) : null}
      {recovery}
      <ActivityLog
        events={events}
        failureAction={(event) => ({
          // Prepared, not sent. The composer is where a request becomes a
          // request, and the reader may want to change it first.
          label: `Ask ${AGENTS.find((a) => a.id === agentId)?.name ?? "the agent"} to fix this`,
          onAct: () =>
            setDraft(
              `${event.summary} failed. Read the output above, find the cause, and fix it.`,
            ),
        })}
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
      {review || (diff && diff.files.length > 0) ? (
        <div className="border-b border-border">
          <ReviewFindings
            busyFindingId={busyFindingId}
            onRunReview={() => void runReview()}
            reviewerLabel={agentId === "codex" ? "Claude" : "Codex"}
            implementerLabel={AGENTS.find((agent) => agent.id === agentId)?.name}
            onAskForFix={(finding) =>
              setDraft(
                `${finding.relativePath}:${finding.line} — ${finding.title}\n\n` +
                  `${finding.body}\n\nFix this.`,
              )
            }
            onDecide={(finding, status) => void decideFinding(finding, status)}
            onOpenFinding={(path, line) => void openFile(path, false, line)}
            currentDiffSha={diff?.taskId === selectedTaskId ? diff.diffSha256 : null}
            review={review}
            state={reviewState}
          />
        </div>
      ) : null}

      {projection && diff && diff.files.length > 0 ? (
        <ApprovalGate
          branchName={diff.branchName}
          onApprove={(noteText) => void approve(noteText)}
          outstanding={(review?.findings ?? []).filter(
            (finding) => finding.status === "open" || finding.status === "sent",
          )}
          projection={projection}
        />
      ) : null}

      {drift ? (
        <BaseDriftMessage
          busy={driftBusy}
          drift={drift}
          onResolve={(choice) => void resolveDrift(choice)}
        />
      ) : null}

      {pullRequest ? (
        <PullRequestActivity
          onOpenFile={(path, line) => void openFile(path, false, line)}
          onOpenUrl={(url) => void invoke("open_external_url", { url }).catch(() => undefined)}
          onRefresh={() => {
            if (!selectedTaskId) return;
            setPullRequestState("loading");
            invoke<PullRequestSnapshot>("refresh_github_pull_request", { taskId: selectedTaskId })
              .then((snapshot) => {
                setPullRequest(snapshot);
                setPullRequestState("idle");
              })
              .catch(() => setPullRequestState("failed"));
          }}
          onReplyWithFix={(comment) =>
            setDraft(
              `${comment.author} wrote on ${comment.path ?? "the pull request"}` +
                `${comment.line === null ? "" : `:${comment.line}`}:\n\n` +
                `${comment.body}\n\nAnswer this with a fix.`,
            )
          }
          snapshot={pullRequest}
          state={pullRequestState}
        />
      ) : null}

      {publish ? (
        <PublishPreview
          busy={publish.busy}
          checks={publish.checks}
          commits={publish.commits}
          error={publish.error}
          onPublish={(plan) => void doPublish(plan)}
          onRevise={(next) => {
            if (selectedTaskId) void planPublish(selectedTaskId, next);
          }}
          plan={publish.plan}
          state={publish.state}
        />
      ) : null}

      {ask ? (
        <BrainAnswerMessage
          answer={ask.answer}
          error={ask.error}
          key={ask.id}
          onOpenCitation={(path, line) => void openFile(path, false, line)}
          question={ask.question}
          shas={askShas}
          stage={ask.stage}
          state={ask.state}
          streamed={ask.streamed}
          streamedCitations={ask.streamedCitations}
        />
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
  const hasRecord =
    events.length > 0 ||
    notes.length > 0 ||
    recovery.length > 0 ||
    ask !== null ||
    publish !== null ||
    drift !== null ||
    pullRequest !== null;
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

  if (focus) {
    return (
      <FocusMode
        branchName={selectedTask?.branchName}
        conversationState={activeRun ? runActivityState(activeRun) : "idle"}
        editor={
          activeFile ? (
            <Suspense fallback={<StateBlock state="loading" title="Loading editor" />}>
              <CodeEditor
                key={activeFile.document.relativePath}
                document={activeFile.document}
                onChange={(value) => updateActive((file) => ({ ...file, draft: value }))}
                onReload={() => void openFile(activeFile.document.relativePath, true)}
                onSaved={(document) => {
                  updateActive(() => ({ document, draft: document.content }));
                  // The save is already in the record; re-read so the count in
                  // the bar above is true rather than merely plausible. The
                  // file just changed, so anything anchored to it is re-checked
                  // too.
                  setTreeRevision((revision) => revision + 1);
                  if (selectedTaskId) void loadRecord(selectedTaskId);
                }}
                repositoryId={project.repositoryId}
                revealLine={revealLine}
                taskId={selectedTaskId}
                value={activeFile.draft}
              />
            </Suspense>
          ) : (
            <StateBlock
              state="idle"
              title="No file open"
              description="Pick a file on the left. Everything you save here lands in the task's activity record."
            />
          )
        }
        explorer={
          <FileExplorer
            activePath={activePath}
            onOpen={(path) => void openFile(path)}
            repositoryId={project.repositoryId}
            repositoryName={project.name}
          />
        }
        onReturn={() => setFocus(false)}
        onToggleTerminal={() => setFocusTerminal((open) => !open)}
        openPath={activePath}
        returnLabel={selectedTask?.title ?? "the conversation"}
        terminal={<TerminalPanel project={project} />}
        terminalOpen={focusTerminal}
        unread={Math.max(0, events.length - focusFrom)}
      />
    );
  }

  return (
    <>
      <CommandPalette
        hits={paletteHits}
        onClose={() => setPaletteOpen(false)}
        onJump={jumpTo}
        onSearch={searchPalette}
        open={paletteOpen}
        state={paletteState}
      />
      <LibraryOverlay
        memories={
          <MemoryBrowser
            busyId={busyMemoryId}
            filters={memoryFilters}
            memories={memories}
            onApprove={(proposal, editedText) =>
              void memoryAction(proposal.id, () =>
                invoke("memory_approve", {
                  request: { proposalId: proposal.id, editedText },
                }),
              )
            }
            onFilters={setMemoryFilters}
            onJumpToOrigin={(conversationId) => {
              // The conversation a memory came from is a task's log.
              setSelectedTaskId(conversationId);
              setLibrary(null);
              dispatch({ type: "navigate", destination: "conversation" });
            }}
            onReject={(proposal, reason) =>
              void memoryAction(proposal.id, () =>
                invoke("memory_reject", { request: { proposalId: proposal.id, reason } }),
              )
            }
            onRevise={(memory, text) =>
              void memoryAction(memory.id, () =>
                invoke("memory_revise", { request: { memoryId: memory.id, text } }),
              )
            }
            onRevoke={(memory, reason) =>
              void memoryAction(memory.id, () =>
                invoke("memory_revoke", { request: { memoryId: memory.id, reason } }),
              )
            }
            proposals={proposals}
            state={memoryState}
          />
        }
        onClose={() => setLibrary(null)}
        onTab={setLibrary}
        open={library !== null}
        settings={
          <Settings
            agents={agentProbes}
            agentsState={probeState}
            busyGrantId={busyGrantId}
            connection={null}
            grants={grants}
            grantsState={grantsState}
            index={index}
            onConnectGitHub={() =>
              note("Connecting GitHub needs a token; that flow is not on this screen yet.", "queued")
            }
            onReindex={() => {
              setIndexState("loading");
              invoke("index_repository", { repositoryId: project.repositoryId })
                .then(loadIndex)
                .catch(() => setIndexState("failed"));
            }}
            onRevokeGrant={(grant) => void revokeGrant(grant)}
            repositoryName={project.name}
            storage={null}
          />
        }
        skills={
          <SkillsLibrary
            busyId={null}
            onRemove={(skill) =>
              void invoke("remove_skill_installation", { installationId: skill.id })
                .then(() => setReloadToken((token) => token + 1))
                .catch((caught: unknown) =>
                  note(`The skill could not be removed: ${messageFor(caught)}`, "failed"),
                )
            }
            onToggleAdapter={() =>
              note(
                "Turning a skill on or off per agent is not wired to a command yet; the skill's " +
                  "text and scope above are read from what is installed.",
                "queued",
              )
            }
            skills={skills}
            state={taskSkills ? "idle" : "loading"}
          />
        }
        tab={library ?? "memories"}
      />
      <ShellFrame
      accountLabel="Local profile"
      onAccount={() => setLibrary("memories")}
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
        "source-control": {
          content: (
            <SourceControl
              diff={
                <DiffReview
                  branchName={diff?.branchName}
                  error={diffError}
                  files={diff?.taskId === selectedTaskId ? diff.files : []}
                  headOid={diff?.headOid}
                  onComment={(path, line, text, side) => void comment(path, line, text, side)}
                  onOpenFile={(path, line) => void openFile(path, false, line)}
                  onReread={() => {
                    if (selectedTask && selectedTaskId) {
                      void loadDiff(selectedTaskId, selectedTask.branchName);
                    }
                  }}
                  staleReason={diffStaleReason}
                  state={diffState}
                />
              }
              onSelectTask={setSelectedTaskId}
              selectedTaskId={selectedTaskId}
              tasks={tasks}
              worktreeState={worktreeActivityState}
            />
          ),
          state: diffError ? "failed" : diffStaleReason ? "stale" : "idle",
          detail: diff?.taskId === selectedTaskId ? `${diff.files.length} file(s)` : undefined,
        },
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
            <BrainExplorer
              onExplore={(target) => runSearch(target)}
              onOpen={(path, line) => void openFile(path, false, line)}
              onSearch={runSearch}
              results={searchResults}
              searchState={searchState}
              slice={null}
              sliceState="idle"
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
        content: {
          terminal: <TerminalPanel project={project} />,
          problems: (
            <ProblemsPanel
              onOpen={(path, line) => void openFile(path, false, line)}
              problems={problems}
              state={eventsState}
            />
          ),
          tests: (
            <TestsPanel
              onOpenOutput={(digest) => {
                void loadPayload(digest).then((payload) =>
                  note(payload.text.slice(0, 4000), "done"),
                );
              }}
              onRerun={(command) => setDraft(`Run ${command} again and report what changed.`)}
              projection={projection}
              state={eventsState}
            />
          ),
          checks: (
            <ChecksPanel
              checks={pullRequest?.checks ?? []}
              onOpenUrl={(url) => void invoke("open_external_url", { url }).catch(() => undefined)}
              onRefresh={() => setReloadToken((token) => token + 1)}
              state={pullRequestState}
            />
          ),
        },
        statuses: {
          problems:
            problems.length > 0 ? { state: "failed", count: problems.length } : undefined,
          tests: projection
            ? {
                state: projection.tests.some((test) => test.status === "failed")
                  ? "failed"
                  : "idle",
                count: projection.tests.length,
              }
            : undefined,
          checks: pullRequest ? { state: "idle", count: pullRequest.checks.length } : undefined,
        },
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
    </>
  );
}
