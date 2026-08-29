import type { CommandFailure, ProjectSnapshot, WorktreePlan } from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { TaskDetail } from "@/features/tasks/TaskDetail";
import { TaskList } from "@/features/tasks/TaskList";

interface TaskWorkspaceProps {
  onOpenWorktree: (project: ProjectSnapshot) => void;
  repositoryId: string;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

export function TaskWorkspace({ onOpenWorktree, repositoryId }: TaskWorkspaceProps) {
  const [tasks, setTasks] = useState<WorktreePlan[]>([]);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    invoke<WorktreePlan[]>("list_tasks", { repositoryId })
      .then((items) => {
        if (!active) return;
        setTasks(items);
        setSelectedTaskId((current) => current ?? items.at(0)?.taskId ?? null);
      })
      .catch((caught) => {
        if (active) setError(messageFor(caught));
      });
    return () => {
      active = false;
    };
  }, [repositoryId]);

  const selectedTask = useMemo(
    () => tasks.find((task) => task.taskId === selectedTaskId) ?? null,
    [selectedTaskId, tasks],
  );

  function upsert(task: WorktreePlan) {
    setTasks((current) => [task, ...current.filter((item) => item.taskId !== task.taskId)]);
    setSelectedTaskId(task.taskId);
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <TaskList
        onCreated={upsert}
        onSelect={setSelectedTaskId}
        repositoryId={repositoryId}
        selectedTaskId={selectedTaskId}
        tasks={tasks}
      />
      {selectedTask ? (
        <div className="flex min-h-0 flex-[1.4] border-t">
          <TaskDetail
            key={selectedTask.taskId}
            onOpenWorktree={onOpenWorktree}
            onTaskChanged={upsert}
            task={selectedTask}
          />
        </div>
      ) : null}
      {error ? (
        <Alert className="m-2" variant="destructive">
          <AlertTitle>Tasks unavailable</AlertTitle>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      ) : null}
    </div>
  );
}
