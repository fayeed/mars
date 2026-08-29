import type {
  AdapterKind,
  AdapterProbe,
  AgentLaunchOutcome,
  AgentPermissions,
  AgentTimelinePayload,
  CommandFailure,
  OpenRepositoryOutcome,
  ProjectSnapshot,
  WorktreePlan,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ExternalLinkIcon,
  GitBranchIcon,
  LoaderCircleIcon,
  PlayIcon,
  SendIcon,
  SquareIcon,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { AgentTimeline } from "@/features/agents/AgentTimeline";
import { PermissionPrompt } from "@/features/agents/PermissionPrompt";
import { ContextTray } from "@/features/context/ContextTray";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";

interface TaskDetailProps {
  onOpenWorktree: (project: ProjectSnapshot) => void;
  onTaskChanged: (task: WorktreePlan) => void;
  task: WorktreePlan;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

export function TaskDetail({ onOpenWorktree, onTaskChanged, task }: TaskDetailProps) {
  const [adapter, setAdapter] = useState<AdapterKind>("claude");
  const [prompt, setPrompt] = useState(task.intent);
  const [probe, setProbe] = useState<AdapterProbe | null>(null);
  const [permissionOpen, setPermissionOpen] = useState(false);
  const [events, setEvents] = useState<AgentTimelinePayload["event"][]>([]);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [providerSessionId, setProviderSessionId] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sessionRef = useRef<string | null>(null);
  const launchPendingRef = useRef(false);

  useEffect(() => {
    let mounted = true;
    let unlisten: (() => void) | undefined;
    void listen<AgentTimelinePayload>("agent-event", ({ payload }) => {
      if (!mounted) return;
      if (sessionRef.current && payload.sessionId !== sessionRef.current) return;
      if (!sessionRef.current && !launchPendingRef.current) return;
      if (!sessionRef.current) {
        sessionRef.current = payload.sessionId;
        setSessionId(payload.sessionId);
      }
      setEvents((current) => [...current, payload.event].slice(-500));
      if (payload.event.type === "checkpoint") setProviderSessionId(payload.event.sessionId);
      if (payload.event.type === "end") {
        launchPendingRef.current = false;
        setRunning(false);
      }
    }).then((dispose) => {
      if (mounted) unlisten = dispose;
      else dispose();
    });
    return () => {
      mounted = false;
      unlisten?.();
    };
  }, [task.taskId]);

  async function provision() {
    setBusy(true);
    setError(null);
    try {
      onTaskChanged(await invoke<WorktreePlan>("provision_task", { taskId: task.taskId }));
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function reviewPermissions() {
    if (!prompt.trim()) return;
    setBusy(true);
    setError(null);
    try {
      setProbe(await invoke<AdapterProbe>("probe_agent", { adapter }));
      setPermissionOpen(true);
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function launch(permissions: AgentPermissions) {
    setPermissionOpen(false);
    setBusy(true);
    setError(null);
    launchPendingRef.current = true;
    sessionRef.current = null;
    setSessionId(null);
    setRunning(true);
    try {
      const outcome = await invoke<AgentLaunchOutcome>("launch_agent", {
        taskId: task.taskId,
        adapter,
        prompt: prompt.trim(),
        resumeSessionId: providerSessionId,
        permissions,
      });
      sessionRef.current = outcome.sessionId;
      setSessionId(outcome.sessionId);
      setPrompt("");
    } catch (caught) {
      launchPendingRef.current = false;
      setRunning(false);
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function cancel() {
    if (!sessionId) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("cancel_agent", { sessionId });
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function openWorktree() {
    setBusy(true);
    setError(null);
    try {
      const result = await invoke<OpenRepositoryOutcome>("open_task_worktree", {
        taskId: task.taskId,
      });
      if (result.status === "opened") onOpenWorktree(result.repository);
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  if (task.state === "planned") {
    return (
      <section className="grid gap-4 p-4" aria-label="Task plan">
        <TaskHeading task={task} />
        <div className="grid gap-3 rounded-lg border bg-muted/20 p-3 text-xs">
          <PlanRow label="Branch" value={task.branchName} />
          <PlanRow label="Base" value={task.baseRef} />
          <PlanRow label="Worktree" value={task.worktreePath} />
        </div>
        <p className="text-xs text-muted-foreground">
          Approval creates the branch and isolated worktree at exactly the locations above.
        </p>
        <Button disabled={busy} onClick={() => void provision()} type="button">
          {busy ? <LoaderCircleIcon className="animate-spin" /> : <GitBranchIcon />}
          Approve worktree plan
        </Button>
        <TaskError error={error} />
      </section>
    );
  }

  return (
    <ScrollArea className="min-h-0 flex-1">
      <section className="grid gap-4 p-4" aria-label="Task agent">
        <TaskHeading task={task} />
        <ContextTray repositoryId={task.repositoryId} taskId={task.taskId} />
        <Button className="justify-start" disabled={busy} onClick={() => void openWorktree()} type="button" variant="outline">
          <ExternalLinkIcon /> Open worktree in editor and terminal
        </Button>

        <div className="grid gap-3 rounded-lg border p-3">
          <div className="flex items-end gap-2">
            <div className="grid flex-1 gap-1.5">
              <Label htmlFor="agent-adapter">Agent CLI</Label>
              <Select onValueChange={(value) => setAdapter(value as AdapterKind)} value={adapter}>
                <SelectTrigger className="w-full" id="agent-adapter">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="claude">Claude Code</SelectItem>
                  <SelectItem value="codex">Codex CLI</SelectItem>
                </SelectContent>
              </Select>
            </div>
            {probe ? <Badge variant="outline">{probe.version}</Badge> : null}
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="agent-prompt">{providerSessionId ? "Follow-up" : "Assignment"}</Label>
            <Textarea
              id="agent-prompt"
              onChange={(event) => setPrompt(event.target.value)}
              placeholder={providerSessionId ? "Steer the existing provider session…" : "Describe the task…"}
              rows={4}
              value={prompt}
            />
          </div>
          <div className="flex gap-2">
            <Button disabled={busy || running || !prompt.trim()} onClick={() => void reviewPermissions()} type="button">
              {busy ? <LoaderCircleIcon className="animate-spin" /> : providerSessionId ? <SendIcon /> : <PlayIcon />}
              {providerSessionId ? "Review follow-up" : "Review permissions"}
            </Button>
            {running ? (
              <Button disabled={busy} onClick={() => void cancel()} type="button" variant="destructive">
                <SquareIcon /> Cancel agent
              </Button>
            ) : null}
          </div>
        </div>

        <AgentTimeline events={events} />
        <TaskError error={error} />
      </section>
      <PermissionPrompt
        adapter={adapter}
        onApprove={(permissions) => void launch(permissions)}
        onOpenChange={setPermissionOpen}
        open={permissionOpen}
        probe={probe}
      />
    </ScrollArea>
  );
}

function TaskHeading({ task }: { task: WorktreePlan }) {
  return (
    <header className="grid gap-1">
      <div className="flex items-start justify-between gap-2">
        <h2 className="font-heading text-lg leading-tight">{task.title}</h2>
        <Badge variant="outline">{task.state}</Badge>
      </div>
      <p className="text-xs leading-relaxed text-muted-foreground">{task.intent}</p>
    </header>
  );
}

function PlanRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid grid-cols-[64px_1fr] gap-2">
      <span className="text-muted-foreground">{label}</span>
      <code className="break-all">{value}</code>
    </div>
  );
}

function TaskError({ error }: { error: string | null }) {
  if (!error) return null;
  return (
    <Alert variant="destructive">
      <AlertTitle>Task operation failed</AlertTitle>
      <AlertDescription>{error}</AlertDescription>
    </Alert>
  );
}
