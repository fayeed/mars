import type { CommandFailure, ForgeConnection, StoredIssue, WorktreePlan } from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { GitPullRequestIcon, LoaderCircleIcon } from "lucide-react";
import { type FormEvent, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

interface ImportGitHubIssueDialogProps {
  onCreated: (task: WorktreePlan) => void;
  repositoryId: string;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

export function ImportGitHubIssueDialog({ onCreated, repositoryId }: ImportGitHubIssueDialogProps) {
  const [open, setOpen] = useState(false);
  const [issueNumber, setIssueNumber] = useState("");
  const [remoteName, setRemoteName] = useState("origin");
  const [baseRef, setBaseRef] = useState("main");
  const [token, setToken] = useState("");
  const [issue, setIssue] = useState<StoredIssue | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function changeOpen(next: boolean) {
    setOpen(next);
    if (!next) {
      setToken("");
      setIssue(null);
      setError(null);
    }
  }

  async function inspect(event: FormEvent) {
    event.preventDefault();
    const number = Number(issueNumber);
    if (!Number.isSafeInteger(number) || number < 1 || !token.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await invoke<ForgeConnection>("connect_github", {
        repositoryId,
        remoteName: remoteName.trim() || "origin",
        token,
      });
      setIssue(await invoke<StoredIssue>("import_github_issue", {
        repositoryId,
        remoteName: remoteName.trim() || "origin",
        issueNumber: number,
      }));
      setToken("");
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function createTask() {
    if (!issue) return;
    setBusy(true);
    setError(null);
    try {
      const task = await invoke<WorktreePlan>("create_task_from_issue", {
        issueId: issue.id,
        baseRef: baseRef.trim() || "main",
      });
      onCreated(task);
      setIssueNumber("");
      changeOpen(false);
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogTrigger render={<Button size="sm" type="button" variant="outline" />}>
        <GitPullRequestIcon data-icon="inline-start" /> Import issue
      </DialogTrigger>
      <DialogContent className="sm:max-w-lg">
        <form className="grid gap-4" onSubmit={(event) => void inspect(event)}>
          <DialogHeader>
            <DialogTitle>GitHub issue → isolated task</DialogTitle>
            <DialogDescription>
              Inspect the issue first. The token is stored by your OS keychain; SQLite keeps only an opaque reference.
            </DialogDescription>
          </DialogHeader>
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1.5">
              <Label htmlFor="github-remote">Remote</Label>
              <Input id="github-remote" onChange={(event) => setRemoteName(event.target.value)} value={remoteName} />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="github-issue">Issue number</Label>
              <Input id="github-issue" inputMode="numeric" onChange={(event) => setIssueNumber(event.target.value)} value={issueNumber} />
            </div>
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="github-token">GitHub access token</Label>
            <Input autoComplete="off" id="github-token" onChange={(event) => setToken(event.target.value)} type="password" value={token} />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="github-base">Task base branch</Label>
            <Input id="github-base" onChange={(event) => setBaseRef(event.target.value)} value={baseRef} />
          </div>
          {issue ? (
            <div className="grid gap-2 rounded-lg border bg-muted/20 p-3">
              <div className="flex items-center gap-2">
                <Badge variant="outline">#{issue.number}</Badge>
                <span className="text-sm font-medium">{issue.title}</span>
              </div>
              <p className="line-clamp-4 text-xs text-muted-foreground">{issue.body || "No issue description."}</p>
            </div>
          ) : null}
          {error ? <p className="text-xs text-destructive">{error}</p> : null}
          <DialogFooter>
            <Button disabled={busy || !token.trim() || !issueNumber.trim()} type="submit" variant="outline">
              {busy ? <LoaderCircleIcon className="animate-spin" /> : null} Connect and inspect issue
            </Button>
            <Button disabled={busy || !issue} onClick={() => void createTask()} type="button">
              Create task plan
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
