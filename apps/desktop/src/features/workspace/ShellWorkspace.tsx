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
  AdapterKind,
  AdapterProbe,
  AgentAccount,
  AgentTimelinePayload,
  LaunchedRun,
  RunOverview,
  RunRecord,
  IndexEvent,
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
  OpenRepositoryOutcome,
  RemoteForge,
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
import { AgentsBoard } from "@/features/agents/AgentsBoard";
import { AuditExport } from "@/features/activity/AuditExport";
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
import { SkillsMarketplace } from "@/features/skills/SkillsMarketplace";
import { SourceControl, type SourceScope } from "@/features/review/SourceControl";
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
import { routeMessage } from "@/features/workspace/message-route";
import { TerminalPanel, type TerminalCommand } from "@/features/terminal/TerminalPanel";
import type { SettingsAgent } from "@/features/settings/Settings";
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
import { isIndexCold } from "@/shell/failure";
import { useWorkingTreeShas } from "@/lib/working-tree";
import { ShellFrame } from "@/shell/ShellFrame";
import { ShortcutSheet } from "@/shell/ShortcutSheet";
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
  /** The root it was read from: the repository, or one task's worktree. */
  repositoryId: string;
  /** Set when it is a task's copy; a save is then written to that task's record. */
  taskId: string | null;
  /** Unique across roots: the same path in a worktree is a different file. */
  key: string;
}

/** Where a file should be opened from. */
type FileOrigin = "repository" | "task";

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
  const { dispatch, layout } = useShell();
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
  const [activeKey, setActiveKey] = useState<string | null>(null);
  /**
   * Each task's worktree, registered as a root the file service can read and
   * save in. Resolved on first use, so the editor works on the task's copy of
   * a file rather than the repository's.
   */
  const worktreeRoots = useRef<Record<string, string>>({});
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
   * The runs of the selected task, as the core records them. The log says what
   * each run did; this says which provider session a follow-up can resume.
   */
  const [taskRuns, setTaskRuns] = useState<{ taskId: string; runs: RunRecord[] } | null>(null);
  /** Every run still going in any task, for the rail, the status bar and the board. */
  const [liveRuns, setLiveRuns] = useState<RunOverview[]>([]);
  /**
   * Messages written while an agent was still working in their task. They are
   * sent, in order, the moment that run finishes — not dropped, and not sent
   * into the middle of a run that cannot hear them.
   */
  const [queued, setQueued] = useState<{ id: string; taskId: string; text: string }[]>([]);
  const queuedRef = useRef(queued);
  useEffect(() => {
    queuedRef.current = queued;
  }, [queued]);
  /** Every run in the repository, newest first, for the Agents board. */
  const [boardRuns, setBoardRuns] = useState<RunOverview[]>([]);
  const [boardState, setBoardState] = useState<ActivityState>("idle");
  const [stoppingRunId, setStoppingRunId] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  /** Whether the next message starts a new task rather than continuing this one. */
  const [composeNew, setComposeNew] = useState(false);
  const [starting, setStarting] = useState(false);

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
  useEffect(() => {
    recordLimitRef.current = recordLimit;
  }, [recordLimit]);
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
  /**
   * The whole failure, not just its sentence.
   *
   * `messageFor` used to run at the catch site, which threw away the core's
   * code before anything could read it — so every failure on screen was an
   * unclassifiable one, and the reader got `adapter.probe_failed` with no
   * indication of what it meant or whether their work had survived.
   */
  const [diffError, setDiffError] = useState<unknown>(null);

  const [review, setReview] = useState<ReviewRecord | null>(null);
  const [reviewState, setReviewState] = useState<ActivityState>("idle");
  const [busyFindingId, setBusyFindingId] = useState<string | null>(null);

  const [library, setLibrary] = useState<LibraryTab | null>(null);
  // Opening a part of Source control from elsewhere (Settings' "Connect
  // GitHub") remounts it on that part.
  const [sourceOpening, setSourceOpening] = useState<{ scope: SourceScope; nonce: number } | null>(
    null,
  );
  const [githubRemotes, setGithubRemotes] = useState<RemoteForge[]>([]);
  const [agentProbes, setAgentProbes] = useState<SettingsAgent[]>([]);
  const [probeState, setProbeState] = useState<ActivityState>("idle");
  /** An agent's sign-in, waiting to be typed into whichever terminal opens. */
  const [terminalCommand, setTerminalCommand] = useState<TerminalCommand | null>(null);

  const [paletteOpen, setPaletteOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [auditOpen, setAuditOpen] = useState(false);
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
  /**
   * What the index pass is doing, while it does it.
   *
   * A 20,000-file repository is minutes of parsing and embedding. Without this
   * the app goes quiet for the duration, which is indistinguishable from having
   * hung — and there was no way to stop it either.
   */
  const [indexProgress, setIndexProgress] = useState<string | null>(null);
  const [startingPoints, setStartingPoints] = useState<StartingPoint[]>([]);
  const [startingPointsState, setStartingPointsState] = useState<ActivityState>("loading");

  const [proposal, setProposal] = useState<WorktreeProposal | null>(null);
  const [planState, setPlanState] = useState<ActivityState>("idle");
  const [planError, setPlanError] = useState<unknown>(null);

  const [pack, setPack] = useState<RunContextPack | null>(null);
  /** The assignment the pack on screen was built from, which a launch must repeat. */
  const [packAssignment, setPackAssignment] = useState("");
  const [packState, setPackState] = useState<ActivityState>("idle");
  const [packError, setPackError] = useState<unknown>(null);
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

  useEffect(() => {
    const subscription = listen<IndexEvent>("index-event", ({ payload }) => {
      switch (payload.event) {
        case "started":
          setIndexProgress(`reading ${payload.totalFiles.toLocaleString()} files`);
          break;
        case "fileIndexed":
          setIndexProgress(
            `${payload.position.toLocaleString()} of ${payload.totalFiles.toLocaleString()} files`,
          );
          break;
        case "phase":
          setIndexProgress(payload.phase);
          break;
        case "failed":
          setIndexProgress(null);
          void loadIndex();
          break;
        default:
          // Completed or cancelled: the status is authoritative from here.
          setIndexProgress(null);
          void loadIndex();
          break;
      }
    });
    return () => {
      void subscription.then((unlisten) => unlisten());
    };
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
  // Settings names the GitHub account in use; it is read when Settings opens
  // so a connection made in Source control shows there without a restart.
  useEffect(() => {
    if (library !== "settings") return;
    let active = true;
    invoke<RemoteForge[]>("forge_status", { repositoryId: project.repositoryId })
      .then((remotes) => {
        if (active) setGithubRemotes(remotes);
      })
      .catch(() => {
        if (active) setGithubRemotes([]);
      });
    return () => {
      active = false;
    };
  }, [library, project.repositoryId]);

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
        invoke<RunRecord[]>("task_runs", { taskId }),
      ])
        .then(([window, projection, pending, runs]) => {
          setRecord({
            taskId,
            events: window.events,
            earlier: window.earlier,
            projection,
            pending,
          });
          setTaskRuns({ taskId, runs });
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
  // of one record, not to the session. Choosing a task also means the next
  // message is for it, and a pack built for another task is not this one's.
  useEffect(() => {
    setRecordLimit(RECORD_PAGE);
    setComposeNew(false);
    setPack((current) => (current && current.taskId !== selectedTaskId ? null : current));
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

  const boardOpen = layout.destination === "agents";
  const refreshBoard = useCallback(() => {
    setBoardState((current) => (current === "idle" ? current : "loading"));
    return invoke<RunOverview[]>("repository_runs", {
      repositoryId: project.repositoryId,
      limit: 200,
    })
      .then((runs) => {
        setBoardRuns(runs);
        setBoardState("idle");
      })
      .catch(() => setBoardState("failed"));
  }, [project.repositoryId]);

  const refreshLiveRuns = useCallback(
    () =>
      Promise.all([
        invoke<RunOverview[]>("active_runs")
          .then(setLiveRuns)
          .catch(() => undefined),
        boardOpen ? refreshBoard() : Promise.resolve(),
      ]).then(() => undefined),
    [boardOpen, refreshBoard],
  );

  useEffect(() => {
    void refreshLiveRuns();
  }, [refreshLiveRuns]);

  // The board's clock moves only while there is something live to measure.
  const anyLive = liveRuns.some((run) => run.live);
  useEffect(() => {
    if (!boardOpen || !anyLive) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [anyLive, boardOpen]);

  /**
   * Re-read the selected task at most a few times a second.
   *
   * A busy agent emits dozens of events a second. Each is only a nudge — the
   * record is the truth — so a burst of nudges collapses into one read at the
   * end of it rather than one read per event.
   */
  const reloadTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const scheduleReload = useCallback(
    (taskId: string) => {
      if (reloadTimer.current) return;
      reloadTimer.current = setTimeout(() => {
        reloadTimer.current = null;
        void loadRecord(taskId);
      }, 200);
    },
    [loadRecord],
  );
  useEffect(
    () => () => {
      if (reloadTimer.current) clearTimeout(reloadTimer.current);
    },
    [],
  );

  // The agent event is a nudge, not the truth. It says something happened; the
  // record says what, and the record is what gets rendered. Events name their
  // task, so another task's run never disturbs the one on screen.
  useEffect(() => {
    if (!selectedTaskId) return;
    const forThisTask = (taskId: string | undefined) => !taskId || taskId === selectedTaskId;
    const subscriptions = [
      listen<AgentTimelinePayload>("agent-event", ({ payload }) => {
        if (forThisTask(payload.taskId)) scheduleReload(selectedTaskId);
      }),
      // Written by another process — an agent's MCP server asking permission,
      // most often — while the agent's own stream is silent.
      listen<{ taskId: string }>("activity-changed", ({ payload }) => {
        if (forThisTask(payload.taskId)) scheduleReload(selectedTaskId);
      }),
    ];
    return () => {
      for (const subscription of subscriptions) {
        void subscription.then((unlisten) => unlisten());
      }
    };
  }, [scheduleReload, selectedTaskId]);

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
  }, [selectedTaskId, reloadToken]);

  // Skills belong to the task they were installed for; a stale list is never
  // offered for a different one. Only those that reach the selected agent are
  // offered: a switched-off, blocked, or other-agent skill would be a promise
  // the run does not keep.
  const skills = (taskSkills?.taskId === selectedTaskId ? taskSkills.items : []).filter(
    (skill) =>
      skill.enabled &&
      !skill.blocked &&
      (agentId === "codex" ? skill.allowCodex : skill.allowClaude),
  );

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

  /**
   * The agents as the composer should present them: named either way, and the
   * ones this machine does not have marked rather than hidden. Hiding them
   * would make an empty list look like a product with no agents in it.
   */
  const composerAgents = useMemo<ComposerAgent[]>(
    () =>
      AGENTS.map((agent) => {
        const probed = agentProbes.find((candidate) => candidate.id === agent.id);
        if (!probed) return agent;
        if (!probed.probe) {
          return {
            ...agent,
            unavailable:
              "is not installed on this machine — nothing can be launched with it. " +
              "`/ask` still works.",
          };
        }
        if (probed.account?.billing.type === "signedOut") {
          return {
            ...agent,
            unavailable:
              "is installed but not signed in — sign in from Settings → Agents" +
              (probed.account.signInCommand ? ` or with \`${probed.account.signInCommand}\`` : "") +
              ". `/ask` still works.",
          };
        }
        return agent;
      }),
    [agentProbes],
  );

  /**
   * The rail shows what each task is doing now. A task with an agent at work
   * reads as streaming — or blocked when that agent is waiting on an answer —
   * whatever its worktree's lifecycle says, so a glance at the rail tells you
   * which of several agents needs you.
   */
  const shellTasks = useMemo<ShellTask[]>(
    () =>
      tasks.map((task) => {
        const run = liveRuns.find((candidate) => candidate.taskId === task.taskId && candidate.live);
        return {
          id: task.taskId,
          title: task.title,
          state: run
            ? run.pendingPermissions > 0
              ? "blocked"
              : "streaming"
            : worktreeActivityState(task.state),
          branch: task.branchName,
          detail: run
            ? run.pendingPermissions > 0
              ? `${run.adapterKind} · waiting on you`
              : `${run.adapterKind} · working`
            : task.state,
          agent: run?.adapterKind,
        };
      }),
    [liveRuns, tasks],
  );

  /** The root a task's files are read from: its worktree, registered once. */
  const worktreeRoot = useCallback(async (taskId: string): Promise<string | null> => {
    const known = worktreeRoots.current[taskId];
    if (known) return known;
    try {
      const outcome = await invoke<OpenRepositoryOutcome>("open_task_worktree", { taskId });
      if (outcome.status !== "opened") return null;
      worktreeRoots.current[taskId] = outcome.repository.repositoryId;
      return outcome.repository.repositoryId;
    } catch {
      return null;
    }
  }, []);

  /**
   * Open a file in the editor.
   *
   * A file a task's agent changed, or one opened from that task's diff, is
   * the task's copy in its worktree; anything else is the repository's. The
   * two are different files with the same path, and are kept apart.
   */
  const openFile = useCallback(
    async (
      relativePath: string,
      force = false,
      line: number | null = null,
      origin: FileOrigin = "repository",
    ) => {
      setRevealLine(line);
      dispatch({ type: "navigate", destination: "editor" });
      setFileError(null);
      let repositoryId = project.repositoryId;
      let taskId: string | null = null;
      const task = origin === "task" ? selectedTask : null;
      if (task && task.state === "ready") {
        const root = await worktreeRoot(task.taskId);
        if (root) {
          repositoryId = root;
          taskId = task.taskId;
        }
      }
      const key = `${repositoryId}:${relativePath}`;
      const existing = files.find((file) => file.key === key);
      if (existing && !force) {
        setActiveKey(key);
        return;
      }
      try {
        const document = await invoke<FileDocument>("read_file", { repositoryId, relativePath });
        setFiles((current) => [
          ...current.filter((file) => file.key !== key),
          { document, draft: document.content, repositoryId, taskId, key },
        ]);
        setActiveKey(key);
      } catch (caught) {
        setFileError(messageFor(caught));
      }
    },
    [dispatch, files, project.repositoryId, selectedTask, worktreeRoot],
  );

  /** Read an open file again from the root it came from. */
  async function reloadFile(file: OpenFile) {
    setFileError(null);
    try {
      const document = await invoke<FileDocument>("read_file", {
        repositoryId: file.repositoryId,
        relativePath: file.document.relativePath,
      });
      setFiles((current) =>
        current.map((candidate) =>
          candidate.key === file.key ? { ...candidate, document, draft: document.content } : candidate,
        ),
      );
    } catch (caught) {
      setFileError(messageFor(caught));
    }
  }

  const activeFile = files.find((file) => file.key === activeKey) ?? null;
  const activePath = activeFile?.taskId ? null : (activeFile?.document.relativePath ?? null);

  function updateActive(update: (file: OpenFile) => OpenFile) {
    if (!activeKey) return;
    setFiles((current) => current.map((file) => (file.key === activeKey ? update(file) : file)));
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
          setDiffError(caught);
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

  /**
   * Assemble the pack for a provisioned task, excluding what was removed.
   *
   * The assignment is passed rather than read from state: the launch rebuilds
   * the pack from the same inputs and compares digests, so the inputs a pack
   * was built from are kept beside it.
   */
  const buildPack = useCallback(
    async (taskId: string, excluded: string[], forAssignment?: string) => {
      const text = forAssignment ?? assignment;
      setPackState("loading");
      setPackError(null);
      try {
        const built = await invoke<RunContextPack>("build_run_context_pack", {
          request: {
            taskId,
            agentId,
            adapter: agentId === "codex" ? "codex" : "claude",
            role: "implementer",
            assignment: text,
            maxBytes: PACK_BUDGET_BYTES,
            excludedEntityRefs: excluded,
          },
        });
        setPack(built);
        setPackAssignment(text);
        setPackState("idle");
      } catch (caught) {
        setPackError(caught);
        setPackState("failed");
      }
    },
    [agentId, assignment],
  );

  /**
   * Start the first run of a task from the pack on screen.
   *
   * The core rebuilds the pack and compares its digest with the one reviewed;
   * if anything moved in between, the pack is rebuilt here and shown again
   * rather than launched.
   */
  async function startRun(digest: string) {
    if (!pack) return;
    setStarting(true);
    setPackError(null);
    try {
      await invoke<LaunchedRun>("start_run", {
        request: {
          taskId: pack.taskId,
          agentId: pack.agentId,
          adapter: pack.adapter,
          role: pack.role,
          assignment: packAssignment,
          maxBytes: PACK_BUDGET_BYTES,
          excludedEntityRefs: removedRefs,
          digest,
        },
      });
      const taskId = pack.taskId;
      setPack(null);
      setRemovedRefs([]);
      await Promise.all([loadRecord(taskId), refreshLiveRuns()]);
    } catch (caught) {
      if ((caught as CommandFailure | null)?.code === "context_pack.changed") {
        note(
          "The context changed after you reviewed it, so nothing was started. The pack below " +
            "is the current one — look it over again.",
          "stale",
        );
        await buildPack(pack.taskId, removedRefs, packAssignment);
      } else {
        setPackError(caught);
      }
    } finally {
      setStarting(false);
    }
  }

  /** Send the person's next message to the session this task already has. */
  async function continueRun(taskId: string, text: string) {
    try {
      await invoke<LaunchedRun>("continue_run", { request: { taskId, text } });
      await Promise.all([loadRecord(taskId), refreshLiveRuns()]);
    } catch (caught) {
      note(`The message could not be sent: ${messageFor(caught)}`, "failed");
      setDraft((current) => current || text);
    }
  }

  async function stopRun(runId: string) {
    setInterrupting(true);
    try {
      await invoke("stop_run", { runId });
      await refreshLiveRuns();
    } catch (caught) {
      note(`The run could not be stopped: ${messageFor(caught)}`, "failed");
    } finally {
      setInterrupting(false);
    }
  }

  // A run starting or ending anywhere changes what the rail, the status bar
  // and the board show; one ending in a task with queued messages sends the
  // next of them.
  const continueRef = useRef(continueRun);
  continueRef.current = continueRun;
  useEffect(() => {
    const subscriptions = [
      listen<{ taskId: string }>("run-started", () => void refreshLiveRuns()),
      listen<{ taskId: string }>("run-finished", ({ payload }) => {
        void refreshLiveRuns();
        const next = queuedRef.current.find((message) => message.taskId === payload.taskId);
        if (next) {
          setQueued((current) => current.filter((message) => message.id !== next.id));
          void continueRef.current(next.taskId, next.text);
        }
      }),
    ];
    return () => {
      for (const subscription of subscriptions) {
        void subscription.then((unlisten) => unlisten());
      }
    };
  }, [refreshLiveRuns]);

  /**
   * Where a message goes.
   *
   * `/ask` is a question about the repository and never becomes work. With a
   * task open, a message belongs to that task: it refines a plan not yet
   * approved, opens the context review for a worktree that has no agent yet,
   * waits its turn behind a run still working, or continues the conversation
   * the task's agent is already in. Without one — or with "new task" chosen —
   * it becomes a new task's plan.
   */
  async function submit(text: string) {
    const task = composeNew ? null : selectedTask;
    const route = routeMessage({
      text,
      composeNew,
      task,
      working: Boolean(task && liveRuns.some((run) => run.taskId === task.taskId && run.live)),
      runs: task && taskRuns?.taskId === task.taskId ? taskRuns.runs : [],
    });
    switch (route.kind) {
      case "ask":
        await askBrain(route.question);
        return;
      case "new-task":
        setComposeNew(false);
        await planNewTask(text);
        return;
      case "queue":
        if (task) {
          setQueued((current) => [
            ...current,
            { id: crypto.randomUUID(), taskId: task.taskId, text },
          ]);
        }
        return;
      case "continue":
        if (task) await continueRun(task.taskId, text);
        return;
      case "refine-plan":
      case "first-run":
        if (!task) return;
        try {
          await invoke("record_message", { taskId: task.taskId, text });
          if (route.kind === "refine-plan") {
            // The plan is not approved yet: what was said refines what will be asked.
            setAssignment((current) => (current ? `${current}

${text}` : text));
            await loadRecord(task.taskId);
          } else {
            // The worktree is ready and no agent has worked in it: the message
            // is the assignment, and the pack it produces is shown before
            // anything runs.
            setAssignment(text);
            await loadRecord(task.taskId);
            await buildPack(task.taskId, [], text);
          }
        } catch (caught) {
          note(`The message could not be recorded: ${messageFor(caught)}`, "failed");
        }
        return;
    }
  }

  /**
   * A request becomes a plan, not a run. Nothing has touched Git at this point:
   * the plan is assembled from reads, shown as a message, and waits.
   */
  async function planNewTask(text: string) {
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
      setPlanError(caught);
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
      await buildPack(provisioned.taskId, [], assignment || provisioned.intent);
    } catch (caught) {
      setPlanError(caught);
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

  /**
   * Which agents this machine actually has.
   *
   * Probed on mount rather than only when Settings is opened. The composer
   * names the agent that will act before a message is written, and it cannot
   * name one honestly without knowing whether it is installed — the alternative
   * was letting somebody compose a whole request and learn at launch that
   * nothing could run it.
   */
  const probeAgents = useCallback(() => {
    setProbeState("loading");
    return Promise.all(
      AGENTS.map(async (agent): Promise<SettingsAgent> => {
        try {
          const probe = await invoke<AdapterProbe>("probe_agent", { adapter: agent.id });
          // Asked separately so an installed agent that cannot say who it is
          // signed in as still reads as installed.
          const account = await invoke<AgentAccount>("agent_account", { adapter: agent.id }).catch(
            () => null,
          );
          return { id: agent.id, name: agent.name, probe, account, error: null };
        } catch (caught) {
          return { id: agent.id, name: agent.name, probe: null, account: null, error: messageFor(caught) };
        }
      }),
    ).then((probes) => {
      setAgentProbes(probes);
      setProbeState("idle");
    });
  }, []);

  useEffect(() => {
    if (agentProbes.length > 0) return;
    void probeAgents();
  }, [agentProbes.length, probeAgents]);

  /**
   * Sign in to an agent: its own command, typed into Mars's terminal.
   *
   * Mars does not sign anyone in. The CLI runs its own sign-in, in the open,
   * and the account it ends up with is never seen here — which is both what the
   * providers require of a tool like this and the only arrangement in which a
   * person's subscription stays theirs.
   */
  async function signInToAgent(adapter: AdapterKind) {
    const known = agentProbes.find((agent) => agent.id === adapter)?.account;
    const account =
      known ?? (await invoke<AgentAccount>("agent_account", { adapter }).catch(() => null));
    if (!account?.signInCommand) {
      note(`Mars could not find how to sign in to ${adapter}. Sign in from your own terminal.`, "failed");
      return;
    }
    setTerminalCommand({ id: Date.now(), text: account.signInCommand });
    if (focus) setFocusTerminal(true);
    else dispatch({ type: "openUtility", tab: "terminal" });
    note(
      `Signing in with \`${account.signInCommand}\` in the terminal. When it finishes, check the agents again in Settings.`,
      "queued",
    );
  }

  /** Keeps Settings and the composer in step with the newest word on an account. */
  const learnAgentAccount = useCallback((account: AgentAccount) => {
    setAgentProbes((current) =>
      current.map((agent) => (agent.id === account.kind ? { ...agent, account } : agent)),
    );
  }, []);

  async function chooseAgentSignIn(adapter: AdapterKind, useSignIn: boolean) {
    try {
      learnAgentAccount(
        await invoke<AgentAccount>("set_agent_sign_in_preference", { adapter, useSignIn }),
      );
    } catch (caught) {
      note(messageFor(caught), "failed");
    }
  }

  const releaseTerminalCommand = useCallback(
    (id: number) => setTerminalCommand((current) => (current?.id === id ? null : current)),
    [],
  );

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
    if (!pack) return;
    const next = included
      ? removedRefs.filter((ref) => ref !== entityRef)
      : [...removedRefs, entityRef];
    setRemovedRefs(next);
    void buildPack(pack.taskId, next, packAssignment);
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
                const current = file.key === activeKey;
                return (
                  <button
                    key={file.key}
                    type="button"
                    aria-current={current ? "page" : undefined}
                    title={
                      file.taskId
                        ? `${file.document.relativePath} in the task's worktree`
                        : file.document.relativePath
                    }
                    onClick={() => setActiveKey(file.key)}
                    className={`relative flex items-center gap-1.5 px-2 text-[11px] outline-none before:absolute before:inset-x-1.5 before:bottom-0 before:h-0.5 before:bg-transparent ${
                      current
                        ? "text-foreground before:bg-primary"
                        : "text-muted-foreground hover:text-foreground"
                    }`}
                  >
                    <Mono>{file.document.relativePath.split("/").at(-1)}</Mono>
                    {file.taskId ? <Mono className="text-faint">task</Mono> : null}
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
                  key={activeFile.key}
                  document={activeFile.document}
                  onChange={(value) => updateActive((file) => ({ ...file, draft: value }))}
                  onReload={() => void reloadFile(activeFile)}
                  onSaved={(document) => {
                    updateActive((file) => ({ ...file, document, draft: document.content }));
                    setTreeRevision((revision) => revision + 1);
                  }}
                  repositoryId={activeFile.repositoryId}
                  revealLine={revealLine}
                  taskId={activeFile.taskId}
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

  const queuedRows = queued
    .filter((message) => message.taskId === selectedTaskId)
    .map((message) => (
      <div className="flex flex-col gap-1 border-b border-border px-3 py-2" key={message.id}>
        <div className="flex items-center gap-2">
          <StateLabel state="queued" detail="sent when the agent finishes" />
          <button
            type="button"
            className="ml-auto text-[11px] text-muted-foreground outline-none hover:text-foreground focus-visible:text-foreground"
            onClick={() =>
              setQueued((current) => current.filter((candidate) => candidate.id !== message.id))
            }
          >
            Withdraw
          </button>
        </div>
        <p className="whitespace-pre-wrap text-[13px] leading-relaxed text-foreground">
          {message.text}
        </p>
      </div>
    ));

  const currentRuns = taskRuns?.taskId === selectedTaskId ? taskRuns.runs : [];
  // A worktree that is ready and has never had an agent in it is one step from
  // work: the context review. Offered here because nothing else on screen
  // would say what to do next.
  const readyForAgent =
    selectedTask?.state === "ready" &&
    currentRuns.length === 0 &&
    !pack &&
    packState !== "loading" &&
    !liveRuns.some((run) => run.taskId === selectedTaskId && run.live);
  const readyRow = readyForAgent ? (
    <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2">
      <StateLabel state="idle" detail="the worktree is ready and no agent has worked in it" />
      <button
        type="button"
        className="ml-auto border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none"
        onClick={() => {
          if (!selectedTask) return;
          const text = assignment || selectedTask.intent;
          setAssignment(text);
          void buildPack(selectedTask.taskId, [], text);
        }}
      >
        Review context and start
      </button>
    </div>
  ) : null;

  const noteRows = notes.map((entry) => (
    <div className="flex flex-col gap-1 border-b border-border px-3 py-2" key={entry.id}>
      <StateLabel state={entry.state} />
      <p className="text-[12px] leading-relaxed text-muted-foreground">{entry.text}</p>
    </div>
  ));

  /**
   * The transcript, given the element it scrolls inside.
   *
   * A function rather than a value because the log virtualizes against that
   * scroller — a 2,000-event conversation renders only the rows near the
   * viewport — and the scroller is the conversation column itself.
   */
  const renderTranscript = (scroller: HTMLElement | null) => (
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
        onOpenFile={(path, line) => void openFile(path, false, line, "task")}
        renderEvent={renderEvent}
        scroller={scroller}
      />
      {pack ? (
        <div className="border-b border-border">
          <RunContextReview
            busy={starting}
            error={packError}
            onStart={(digest) => void startRun(digest)}
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
            onOpenFinding={(path, line) => void openFile(path, false, line, "task")}
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
          onOpenFile={(path, line) => void openFile(path, false, line, "task")}
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

      {readyRow}
      {queuedRows}
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
    pack !== null ||
    readyForAgent ||
    recovery.length > 0 ||
    ask !== null ||
    publish !== null ||
    drift !== null ||
    pullRequest !== null;
  const conversationBody = (scroller: HTMLElement | null) =>
    hasRecord ? (
      renderTranscript(scroller)
    ) : selectedTask ? (
      <TaskDetail
        key={selectedTask.taskId}
        onAccountChanged={learnAgentAccount}
        onOpenWorktree={onOpenProject}
        onSignIn={(adapter) => void signInToAgent(adapter)}
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
                key={activeFile.key}
                document={activeFile.document}
                onChange={(value) => updateActive((file) => ({ ...file, draft: value }))}
                onReload={() => void reloadFile(activeFile)}
                onSaved={(document) => {
                  updateActive((file) => ({ ...file, document, draft: document.content }));
                  // The save is already in the record; re-read so the count in
                  // the bar above is true rather than merely plausible. The
                  // file just changed, so anything anchored to it is re-checked
                  // too.
                  setTreeRevision((revision) => revision + 1);
                  if (selectedTaskId) void loadRecord(selectedTaskId);
                }}
                repositoryId={activeFile.repositoryId}
                revealLine={revealLine}
                taskId={activeFile.taskId}
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
        terminal={
          <TerminalPanel
            command={terminalCommand}
            onCommandSent={releaseTerminalCommand}
            project={project}
          />
        }
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
            connection={
              githubRemotes.find((remote) => remote.connected && remote.name === "origin")
                ?.connection ??
              githubRemotes.find((remote) => remote.connected)?.connection ??
              null
            }
            grants={grants}
            grantsState={grantsState}
            index={index}
            onConnectGitHub={() => {
              setLibrary(null);
              setSourceOpening((current) => ({ scope: "github", nonce: (current?.nonce ?? 0) + 1 }));
              dispatch({ type: "navigate", destination: "source-control" });
            }}
            onRecheckAgents={() => void probeAgents()}
            onSignIn={(adapter) => void signInToAgent(adapter)}
            onUseSignIn={(adapter, useSignIn) => void chooseAgentSignIn(adapter, useSignIn)}
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
          <SkillsMarketplace
            onChanged={() => setReloadToken((token) => token + 1)}
            repositoryId={project.repositoryId}
            taskId={selectedTaskId}
            taskTitle={selectedTask?.title}
          />
        }
        tab={library ?? "memories"}
      />
      <ShortcutSheet onClose={() => setShortcutsOpen(false)} open={shortcutsOpen} />
      <AuditExport
        onClose={() => setAuditOpen(false)}
        open={auditOpen}
        taskId={selectedTaskId}
        taskTitle={selectedTask?.title}
      />
      <ShellFrame
      accountLabel="Local profile"
      onAccount={() => setLibrary("memories")}
      keys={{
        onPalette: () => setPaletteOpen((open) => !open),
        onShortcuts: () => setShortcutsOpen((open) => !open),
        onAudit: () => setAuditOpen((open) => !open),
        // The composer is promised to be one key away, so reaching it takes
        // down whatever is covering it rather than leaving a dialog stranded
        // on screen with the cursor behind it.
        onDismissModals: () => {
          setPaletteOpen(false);
          setShortcutsOpen(false);
          setAuditOpen(false);
          setLibrary(null);
        },
      }}
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
        agents: {
          content: (
            <AgentsBoard
              now={now}
              // The conversation is already beside the board; choosing a task
              // puts it there without leaving the board.
              onOpenTask={setSelectedTaskId}
              onStop={(runId) => {
                setStoppingRunId(runId);
                void stopRun(runId).finally(() => setStoppingRunId(null));
              }}
              runs={boardRuns}
              selectedTaskId={selectedTaskId}
              state={boardState}
              stoppingRunId={stoppingRunId}
            />
          ),
          state: liveRuns.some((run) => run.live && run.pendingPermissions > 0)
            ? "blocked"
            : anyLive
              ? "streaming"
              : boardState,
          detail: `${liveRuns.filter((run) => run.live).length} live`,
        },
        editor: {
          content: editorSurface,
          state: fileError ? "failed" : "idle",
          detail: activePath ?? undefined,
        },
        "source-control": {
          content: (
            <SourceControl
              key={`source-control:${sourceOpening?.nonce ?? 0}`}
              baseBranch={index?.branch ?? "HEAD"}
              initialScope={sourceOpening?.scope}
              onTaskPlanned={(planned) => {
                setProposal(planned);
                setSelectedTaskId(planned.taskId);
                setReloadToken((token) => token + 1);
                void loadRecord(planned.taskId);
                dispatch({ type: "navigate", destination: "conversation" });
              }}
              diff={
                <DiffReview
                  branchName={diff?.branchName}
                  error={diffError}
                  files={diff?.taskId === selectedTaskId ? diff.files : []}
                  headOid={diff?.headOid}
                  onComment={(path, line, text, side) => void comment(path, line, text, side)}
                  onOpenFile={(path, line) => void openFile(path, false, line, "task")}
                  onReread={() => {
                    if (selectedTask && selectedTaskId) {
                      void loadDiff(selectedTaskId, selectedTask.branchName);
                    }
                  }}
                  staleReason={diffStaleReason}
                  state={diffState}
                />
              }
              onChanged={() => {
                setReloadToken((token) => token + 1);
                setTreeRevision((revision) => revision + 1);
              }}
              onOpenFile={(path, line) => void openFile(path, false, line ?? null, "task")}
              onSelectTask={setSelectedTaskId}
              repositoryId={project.repositoryId}
              repositoryName={project.name}
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
        renderChildren: conversationBody,
        composer: (
          <>
            <RunStatusLine
              blockedSummary={pending[0]?.summary}
              interrupting={interrupting}
              latest={latestEvent}
              onInterrupt={() => {
                if (activeRun) void stopRun(activeRun.runId);
              }}
              run={activeRun}
            />
            <Composer
            agents={composerAgents}
            attachments={attachments}
            mentions={mentions}
            onDropFiles={(dropped) => void attachFiles(dropped)}
            onRemoveAttachment={(id) =>
              setAttachments((current) => current.filter((item) => item.id !== id))
            }
            onSelectAgent={setAgentId}
            onSelectScope={setScopeId}
            onStop={() => {
              if (activeRun) void stopRun(activeRun.runId);
            }}
            onSubmit={(submission) => void submit(submission.text)}
            onTargetChange={(mode) => setComposeNew(mode === "new")}
            queuedCount={queued.filter((message) => message.taskId === selectedTaskId).length}
            target={{
              mode: selectedTask && !composeNew ? "task" : "new",
              taskTitle: selectedTask?.title,
            }}
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
              busy={starting}
              error={packError}
              onStart={(digest) => void startRun(digest)}
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
                    onClick={() => void openFile(file.path, false, null, "task")}
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
          terminal: (
            <TerminalPanel
              command={terminalCommand}
              onCommandSent={releaseTerminalCommand}
              project={project}
            />
          ),
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
          state: indexProgress ? "loading" : indexActivityState(index),
          // "0 files" is a number a reader has to interpret. A repository that
          // has not been read says so, and a pass in flight says where it is up
          // to rather than reporting the stale count as if it were current.
          detail:
            indexProgress ??
            (!index
              ? undefined
              : isIndexCold(index)
                ? "not indexed"
                : `${index.counts.files.toLocaleString()} files`),
        },
        agents: {
          state: liveRuns.some((run) => run.live && run.pendingPermissions > 0)
            ? "blocked"
            : liveRuns.some((run) => run.live)
              ? "streaming"
              : "idle",
          count: liveRuns.filter((run) => run.live).length,
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
