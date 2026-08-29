import type { WorktreePlan } from "@mars/shared-types";
import { GitBranchIcon } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { CreateTaskDialog } from "@/features/tasks/CreateTaskDialog";

interface TaskListProps {
  onCreated: (task: WorktreePlan) => void;
  onSelect: (taskId: string) => void;
  repositoryId: string;
  selectedTaskId: string | null;
  tasks: WorktreePlan[];
}

export function TaskList({ onCreated, onSelect, repositoryId, selectedTaskId, tasks }: TaskListProps) {
  return (
    <section className="flex min-h-0 flex-1 flex-col" aria-label="Tasks">
      <header className="flex items-center justify-between gap-2 border-b p-2">
        <span className="text-xs font-medium">Isolated work</span>
        <CreateTaskDialog onCreated={onCreated} repositoryId={repositoryId} />
      </header>
      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-1 p-2">
          {tasks.map((task) => (
            <Button
              className="h-auto items-start justify-start gap-2 px-2 py-2 text-left"
              key={task.taskId}
              onClick={() => onSelect(task.taskId)}
              type="button"
              variant={selectedTaskId === task.taskId ? "secondary" : "ghost"}
            >
              <GitBranchIcon className="mt-0.5 size-3.5 text-primary" />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-xs font-medium">{task.title}</span>
                <span className="block truncate font-mono text-[10px] text-muted-foreground">{task.branchName}</span>
              </span>
              <Badge variant="outline">{task.state}</Badge>
            </Button>
          ))}
          {tasks.length === 0 ? (
            <p className="px-3 py-8 text-center text-xs text-muted-foreground">No tasks yet. Plan isolated work before launching an agent.</p>
          ) : null}
        </div>
      </ScrollArea>
    </section>
  );
}
