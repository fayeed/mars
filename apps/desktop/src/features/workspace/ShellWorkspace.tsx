import type {
  CommandFailure,
  FileDocument,
  FilePage,
  ProjectSnapshot,
  SkillInstallation,
  WorktreePlan,
  WorktreeState,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { lazy, Suspense, useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { BrainAnswer } from "@/features/brain/BrainAnswer";
import { FileExplorer } from "@/features/explorer/FileExplorer";
import { CreateTaskDialog } from "@/features/tasks/CreateTaskDialog";
import { TaskDetail } from "@/features/tasks/TaskDetail";
import { TerminalPanel } from "@/features/terminal/TerminalPanel";
import type { ActivityState } from "@/shell/activity-state";
import { Composer } from "@/shell/Composer";
import type { ComposerAgent, MentionTarget, ShellTask } from "@/shell/model";
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
}

interface OpenFile {
  document: FileDocument;
  draft: string;
}

interface QueuedTurn {
  id: string;
  text: string;
  agentId: string;
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

/**
 * The product composed out of shell regions. No region here supplies its own
 * chrome: the frame owns the rails, headers, panels, states and keymap, and
 * this file only decides what goes in them.
 */
export function ShellWorkspace({ connection, onOpenProject, project }: ShellWorkspaceProps) {
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
  const [queued, setQueued] = useState<QueuedTurn[]>([]);

  useEffect(() => {
    let active = true;
    invoke<WorktreePlan[]>("list_tasks", { repositoryId: project.repositoryId })
      .then((items) => {
        if (!active) return;
        setTasks(items);
        setSelectedTaskId((current) => current ?? items.at(0)?.taskId ?? null);
        setTasksState("idle");
      })
      .catch(() => {
        if (active) setTasksState("failed");
      });
    return () => {
      active = false;
    };
  }, [project.repositoryId, reloadToken]);

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
              <Suspense
                fallback={<StateBlock state="loading" title="Loading editor" />}
              >
                <CodeEditor
                  key={activeFile.document.relativePath}
                  document={activeFile.document}
                  onChange={(draft) => updateActive((file) => ({ ...file, draft }))}
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

  const conversationBody = selectedTask ? (
    <>
      <TaskDetail
        key={selectedTask.taskId}
        onOpenWorktree={onOpenProject}
        onTaskChanged={upsertTask}
        task={selectedTask}
      />
      {queued.length > 0 ? (
        <ul className="border-t border-border" aria-label="Queued turns">
          {queued.map((turn) => (
            <li key={turn.id} className="flex flex-col gap-1 border-b border-border px-3 py-2">
              <StateLabel state="queued" detail={turn.agentId} />
              <p className="text-[13px] whitespace-pre-wrap text-foreground">{turn.text}</p>
              <span className="text-[11px] text-faint">
                Captured. Launching an agent still goes through the context-pack preview.
              </span>
            </li>
          ))}
        </ul>
      ) : null}
    </>
  ) : undefined;

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
        title: selectedTask?.title ?? "Conversation",
        detail: selectedTask?.branchName,
        state: selectedTask ? worktreeActivityState(selectedTask.state) : "idle",
        children: conversationBody,
        emptyTitle: "Select or start a task",
        emptyDescription:
          "A conversation belongs to a task, so every turn has a branch and a worktree to happen in.",
        composer: (
          <Composer
            agents={AGENTS}
            mentions={mentions}
            onSelectAgent={setAgentId}
            onSubmit={(submission) =>
              setQueued((current) => [
                ...current,
                { id: crypto.randomUUID(), text: submission.text, agentId: submission.agentId },
              ])
            }
            selectedAgentId={agentId}
            skills={skills.map((skill) => ({
              id: skill.id,
              name: skill.name,
              description: `${skill.scope} · ${skill.pinnedHash.slice(0, 12)}`,
            }))}
          />
        ),
      }}
      inspector={{
        content: {
          brain: (
            <BrainAnswer
              onOpenSource={(path, line) => void openFile(path, false, line)}
              repositoryId={project.repositoryId}
            />
          ),
        },
      }}
      utility={{
        content: { terminal: <TerminalPanel project={project} /> },
      }}
      status={{
        repository: project.name,
        worktree: selectedTask?.worktreePath,
        branch: selectedTask?.branchName ?? "—",
        agents: { state: "idle", count: 0 },
        permissions: { state: "idle", pending: 0 },
        connection,
      }}
    />
  );
}
