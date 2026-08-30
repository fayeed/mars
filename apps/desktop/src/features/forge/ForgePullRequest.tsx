import type {
  CommandFailure,
  ForgeConnection,
  ForgeMutation,
  PullRequestPlan,
  PullRequestSnapshot,
  WorktreePlan,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import {
  CheckCircle2Icon,
  CircleDotIcon,
  ExternalLinkIcon,
  GitPullRequestIcon,
  HistoryIcon,
  LoaderCircleIcon,
  RefreshCwIcon,
  ShieldAlertIcon,
} from "lucide-react";
import { useEffect, useState } from "react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

interface ForgePullRequestProps {
  onSendFollowup?: (prompt: string) => void;
  task: WorktreePlan;
}

export function ForgePullRequest({ onSendFollowup, task }: ForgePullRequestProps) {
  const [remoteName, setRemoteName] = useState("origin");
  const [title, setTitle] = useState(task.title);
  const [body, setBody] = useState(task.intent);
  const [draft, setDraft] = useState(false);
  const [token, setToken] = useState("");
  const [connectionLogin, setConnectionLogin] = useState<string | null>(null);
  const [pushApproved, setPushApproved] = useState(false);
  const [forgeApproved, setForgeApproved] = useState(false);
  const [plan, setPlan] = useState<PullRequestPlan | null>(null);
  const [snapshot, setSnapshot] = useState<PullRequestSnapshot | null>(null);
  const [mutations, setMutations] = useState<ForgeMutation[]>([]);
  const [busy, setBusy] = useState(false);
  const [historyBusy, setHistoryBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const snapshotId = snapshot?.id;

  useEffect(() => {
    let active = true;
    void invoke<PullRequestSnapshot | null>("get_task_pull_request", { taskId: task.taskId })
      .then((stored) => {
        if (active) setSnapshot(stored);
      })
      .catch((caught) => {
        if (active) setError(messageFor(caught));
      });
    void invoke<ForgeMutation[]>("list_task_forge_mutations", { taskId: task.taskId })
      .then((stored) => {
        if (active) setMutations(stored);
      })
      .catch((caught) => {
        if (active) setError(messageFor(caught));
      });
    return () => { active = false; };
  }, [task.taskId]);

  useEffect(() => {
    if (!snapshotId) return;
    const interval = window.setInterval(() => {
      void invoke<PullRequestSnapshot>("refresh_github_pull_request", { taskId: task.taskId })
        .then(setSnapshot)
        .catch((caught) => setError(messageFor(caught)));
    }, 30_000);
    return () => window.clearInterval(interval);
  }, [snapshotId, task.taskId]);

  function invalidatePlan() {
    setPlan(null);
    setPushApproved(false);
    setForgeApproved(false);
  }

  async function loadMutations() {
    setHistoryBusy(true);
    try {
      setMutations(await invoke<ForgeMutation[]>("list_task_forge_mutations", {
        taskId: task.taskId,
      }));
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setHistoryBusy(false);
    }
  }

  async function preview() {
    setBusy(true);
    setError(null);
    setSnapshot(null);
    try {
      setPlan(await invoke<PullRequestPlan>("preview_pull_request", {
        taskId: task.taskId,
        remoteName: remoteName.trim() || "origin",
        title: title.trim(),
        body: body.trim(),
        draft,
      }));
      setPushApproved(false);
      setForgeApproved(false);
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function create() {
    if (!plan || !pushApproved || !forgeApproved) return;
    setBusy(true);
    setError(null);
    try {
      setSnapshot(await invoke<PullRequestSnapshot>("create_github_pull_request", {
        plan,
        pushApproved: true,
        forgeApproved: true,
      }));
      setPushApproved(false);
      setForgeApproved(false);
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
      void loadMutations();
    }
  }

  async function refresh() {
    if (!snapshot) return;
    setBusy(true);
    setError(null);
    try {
      setSnapshot(await invoke<PullRequestSnapshot>("refresh_github_pull_request", {
        taskId: task.taskId,
      }));
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function connect() {
    if (!token.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const connection = await invoke<ForgeConnection>("connect_github", {
        repositoryId: task.repositoryId,
        remoteName: remoteName.trim() || "origin",
        token,
      });
      setConnectionLogin(connection.accountLogin);
      setToken("");
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="grid gap-3 rounded-lg border p-3" aria-label="GitHub pull request">
      <div className="flex items-center gap-2">
        <GitPullRequestIcon className="size-4" />
        <h3 className="text-sm font-medium">Publish to GitHub</h3>
        {snapshot ? <Badge className="ml-auto" variant="secondary">PR #{snapshot.number}</Badge> : null}
      </div>
      <div className="grid grid-cols-[112px_1fr] gap-2">
        <div className="grid gap-1.5">
          <Label htmlFor={`pr-remote-${task.taskId}`}>Remote</Label>
          <Input
            id={`pr-remote-${task.taskId}`}
            onChange={(event) => { setRemoteName(event.target.value); invalidatePlan(); }}
            value={remoteName}
          />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor={`pr-title-${task.taskId}`}>Title</Label>
          <Input
            id={`pr-title-${task.taskId}`}
            onChange={(event) => { setTitle(event.target.value); invalidatePlan(); }}
            value={title}
          />
        </div>
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor={`pr-body-${task.taskId}`}>Description</Label>
        <Textarea
          id={`pr-body-${task.taskId}`}
          onChange={(event) => { setBody(event.target.value); invalidatePlan(); }}
          rows={3}
          value={body}
        />
      </div>
      <label className="flex items-center gap-2 text-xs">
        <Checkbox
          checked={draft}
          onCheckedChange={(checked) => { setDraft(checked === true); invalidatePlan(); }}
        />
        Create as draft
      </label>
      <Button disabled={busy || !title.trim()} onClick={() => void preview()} type="button" variant="outline">
        {busy ? <LoaderCircleIcon className="animate-spin" /> : <ShieldAlertIcon />}
        Preview push and PR
      </Button>

      {plan || snapshot ? (
        <div className="grid gap-1.5">
          <Label htmlFor={`pr-token-${task.taskId}`}>GitHub access token</Label>
          <Input
            autoComplete="off"
            id={`pr-token-${task.taskId}`}
            onChange={(event) => setToken(event.target.value)}
            type="password"
            value={token}
          />
          <div className="flex items-center gap-2">
            <Button disabled={busy || !token.trim()} onClick={() => void connect()} size="sm" type="button" variant="outline">
              Save to OS keychain
            </Button>
            {connectionLogin ? <Badge variant="secondary">Connected as {connectionLogin}</Badge> : null}
          </div>
          <p className="text-[10px] text-muted-foreground">The OS keychain stores the token; Mars SQLite stores only its opaque reference.</p>
        </div>
      ) : null}

      {plan ? (
        <div className="grid gap-3 rounded-md border bg-muted/20 p-3 text-xs">
          <div className="grid grid-cols-[72px_1fr] gap-1.5">
            <span className="text-muted-foreground">Repository</span><code>{plan.repository.owner}/{plan.repository.name}</code>
            <span className="text-muted-foreground">Push target</span><code className="break-all">{plan.pushUrl}</code>
            <span className="text-muted-foreground">Branches</span><code>{plan.head} → {plan.base}</code>
            <span className="text-muted-foreground">Commits</span><span>{plan.commitsAhead} ahead · worktree {plan.clean ? "clean" : "has uncommitted changes"}</span>
            <span className="text-muted-foreground">Head</span><code>{plan.headSha.slice(0, 12)}</code>
          </div>
          <code className="overflow-x-auto rounded bg-background p-2 text-[10px]">{plan.pushCommand}</code>
          <label className="flex items-start gap-2 leading-relaxed">
            <Checkbox checked={pushApproved} onCheckedChange={(checked) => setPushApproved(checked === true)} />
            I approve pushing this exact commit to the displayed push target and branch.
          </label>
          <label className="flex items-start gap-2 leading-relaxed">
            <Checkbox checked={forgeApproved} onCheckedChange={(checked) => setForgeApproved(checked === true)} />
            I separately approve creating this GitHub pull request with the displayed title, body, head, and base.
          </label>
          <Button disabled={busy || !pushApproved || !forgeApproved} onClick={() => void create()} type="button">
            {busy ? <LoaderCircleIcon className="animate-spin" /> : <GitPullRequestIcon />}
            Push branch and create PR
          </Button>
        </div>
      ) : null}

      {snapshot ? (
        <>
          <PullRequestResult onSendFollowup={onSendFollowup} snapshot={snapshot} />
          <Button disabled={busy} onClick={() => void refresh()} type="button" variant="outline">
            {busy ? <LoaderCircleIcon className="animate-spin" /> : <CircleDotIcon />}
            Refresh checks and comments
          </Button>
        </>
      ) : null}
      <PublishHistory
        busy={historyBusy}
        mutations={mutations}
        onRefresh={() => void loadMutations()}
      />
      {error ? <Alert variant="destructive"><AlertDescription>{error}</AlertDescription></Alert> : null}
    </section>
  );
}

function PublishHistory({
  busy,
  mutations,
  onRefresh,
}: {
  busy: boolean;
  mutations: ForgeMutation[];
  onRefresh: () => void;
}) {
  return (
    <section aria-label="Publish history" className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <HistoryIcon />
        <h4 className="text-xs font-medium">Publish history</h4>
        <Badge className="ml-auto" variant="outline">{mutations.length}</Badge>
        <Button
          aria-label="Refresh publish history"
          disabled={busy}
          onClick={onRefresh}
          size="icon-sm"
          type="button"
          variant="ghost"
        >
          <RefreshCwIcon className={busy ? "animate-spin" : undefined} />
        </Button>
      </div>
      {mutations.length ? (
        <div className="flex flex-col gap-2">
          {mutations.map((mutation) => (
            <MutationEntry key={mutation.id} mutation={mutation} />
          ))}
        </div>
      ) : (
        <Empty className="min-h-24 p-3">
          <EmptyHeader>
            <EmptyMedia variant="icon"><HistoryIcon /></EmptyMedia>
            <EmptyTitle>No publish attempts</EmptyTitle>
            <EmptyDescription>Reviewed GitHub publish attempts will appear here.</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}
    </section>
  );
}

function MutationEntry({ mutation }: { mutation: ForgeMutation }) {
  const destructive = mutation.status === "failed";
  const stage = mutationStage(mutation.detailsJson);
  return (
    <Alert variant={destructive ? "destructive" : "default"}>
      {mutation.status === "completed" ? <CheckCircle2Icon /> : mutation.status === "started" ? <LoaderCircleIcon className="animate-spin" /> : <ShieldAlertIcon />}
      <AlertTitle className="flex items-center gap-2">
        Pull request publish
        <Badge variant={mutationBadge(mutation.status)}>{mutation.status}</Badge>
      </AlertTitle>
      <AlertDescription>
        <div className="flex flex-col gap-1">
          <span>{stage} · <time dateTime={mutation.startedAt}>{formatTimestamp(mutation.startedAt)}</time></span>
          <code className="text-[10px]">Plan {mutation.planFingerprint.slice(0, 12)}</code>
          <span className="text-[10px]">Push approved · forge approved</span>
          {mutation.error ? <span>{mutation.error}</span> : null}
        </div>
      </AlertDescription>
    </Alert>
  );
}

function mutationBadge(status: ForgeMutation["status"]): "default" | "secondary" | "destructive" | "outline" {
  if (status === "completed") return "secondary";
  if (status === "failed") return "destructive";
  if (status === "interrupted") return "outline";
  return "default";
}

function mutationStage(detailsJson: string) {
  try {
    const details = JSON.parse(detailsJson) as { stage?: unknown };
    return typeof details.stage === "string" ? details.stage : "recorded";
  } catch {
    return "recorded";
  }
}

function formatTimestamp(timestamp: string) {
  const parsed = new Date(timestamp);
  return Number.isNaN(parsed.valueOf()) ? timestamp : parsed.toLocaleString();
}

function PullRequestResult({
  onSendFollowup,
  snapshot,
}: {
  onSendFollowup?: (prompt: string) => void;
  snapshot: PullRequestSnapshot;
}) {
  return (
    <div className="grid gap-3 rounded-md border p-3 text-xs">
      <a className="flex items-center gap-2 font-medium hover:underline" href={snapshot.url} rel="noreferrer" target="_blank">
        {snapshot.title} <ExternalLinkIcon className="size-3" />
      </a>
      <div className="grid gap-1.5">
        <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">Checks</span>
        {snapshot.checks.length ? snapshot.checks.map((check) => (
          <a className="flex items-center gap-2 hover:underline" href={check.url} key={check.providerId} rel="noreferrer" target="_blank">
            {check.conclusion === "success" ? <CheckCircle2Icon className="size-3.5 text-emerald-600" /> : <CircleDotIcon className="size-3.5" />}
            {check.name} · {check.conclusion ?? check.status}
          </a>
        )) : <span className="text-muted-foreground">No checks reported yet.</span>}
      </div>
      <div className="grid gap-1.5">
        <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">Review comments</span>
        {snapshot.comments.length ? snapshot.comments.map((comment) => (
          <article className="grid gap-2 rounded border bg-muted/15 p-2" key={comment.providerId}>
            <a className="hover:underline" href={comment.url} rel="noreferrer" target="_blank">
              <span className="font-medium">{comment.author}</span>
              {comment.path ? <span className="text-muted-foreground"> · {comment.path}{comment.line ? `:${comment.line}` : ""}</span> : null}
              <p className="mt-1">{comment.body}</p>
            </a>
            {onSendFollowup ? (
              <Button
                className="justify-self-start"
                onClick={() => onSendFollowup([
                  "Address this GitHub review comment.",
                  comment.path ? `Location: ${comment.path}${comment.line ? `:${comment.line}` : ""}` : "",
                  `Reviewer: ${comment.author}`,
                  comment.body,
                  `Source: ${comment.url}`,
                ].filter(Boolean).join("\n"))}
                size="sm"
                type="button"
                variant="outline"
              >
                Send to agent
              </Button>
            ) : null}
          </article>
        )) : <span className="text-muted-foreground">No review comments yet.</span>}
      </div>
    </div>
  );
}
