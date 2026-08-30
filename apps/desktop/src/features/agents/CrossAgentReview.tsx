import type {
  AdapterKind,
  CommandFailure,
  FindingStatus,
  ReviewFinding,
  ReviewHandoff,
  ReviewRecord,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowRightIcon,
  CheckCircle2Icon,
  GitCompareArrowsIcon,
  LoaderCircleIcon,
  RefreshCwIcon,
  SendIcon,
  ShieldCheckIcon,
  TriangleAlertIcon,
} from "lucide-react";
import { useState } from "react";

import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";

interface CrossAgentReviewProps {
  disabled?: boolean;
  implementationAdapter: AdapterKind;
  onSendFollowup: (prompt: string, adapter: AdapterKind) => void;
  taskId: string;
}

export function CrossAgentReview({
  disabled = false,
  implementationAdapter,
  onSendFollowup,
  taskId,
}: CrossAgentReviewProps) {
  const reviewerAdapter: AdapterKind = implementationAdapter === "claude" ? "codex" : "claude";
  const [open, setOpen] = useState(false);
  const [handoff, setHandoff] = useState<ReviewHandoff | null>(null);
  const [reviews, setReviews] = useState<ReviewRecord[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function openReview() {
    setOpen(true);
    setBusy(true);
    setError(null);
    try {
      setReviews(await invoke<ReviewRecord[]>("list_task_reviews", { taskId }));
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function prepare() {
    setBusy(true);
    setError(null);
    try {
      setHandoff(await invoke<ReviewHandoff>("build_review_handoff", {
        taskId,
        reviewerAdapter,
        maxBytes: 64 * 1024,
      }));
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function runReview() {
    if (!handoff) return;
    setBusy(true);
    setError(null);
    try {
      const review = await invoke<ReviewRecord>("run_cross_agent_review", {
        handoff,
        modelPermission: true,
      });
      setReviews((current) => [review, ...current.filter((item) => item.id !== review.id)]);
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function setFindingStatus(finding: ReviewFinding, status: FindingStatus) {
    const updated = await invoke<ReviewFinding>("set_review_finding_status", {
      findingId: finding.id,
      status,
    });
    setReviews((current) => current.map((review) => ({
      ...review,
      findings: review.findings.map((item) => item.id === updated.id ? updated : item),
    })));
  }

  async function sendFollowup(review: ReviewRecord) {
    setBusy(true);
    setError(null);
    try {
      const prompt = await invoke<string>("review_followup", { reviewId: review.id });
      await Promise.all(review.findings
        .filter((finding) => finding.status === "open")
        .map((finding) => setFindingStatus(finding, "sent")));
      onSendFollowup(prompt, implementationAdapter);
      setOpen(false);
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Button disabled={disabled || busy} onClick={() => void openReview()} type="button" variant="outline">
        <GitCompareArrowsIcon /> Review with {agentLabel(reviewerAdapter)}
      </Button>
      <Dialog onOpenChange={setOpen} open={open}>
        <DialogContent className="max-h-[92vh] sm:max-w-4xl">
          <DialogHeader>
            <DialogTitle>Cross-agent review</DialogTitle>
            <DialogDescription>
              {agentLabel(implementationAdapter)} implementation → read-only {agentLabel(reviewerAdapter)} review. The exact handoff is visible before model use.
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-wrap gap-2">
            <Button disabled={busy} onClick={() => void prepare()} size="sm" type="button" variant={handoff ? "outline" : "default"}>
              {busy ? <LoaderCircleIcon className="animate-spin" /> : <RefreshCwIcon />}
              {handoff ? "Refresh handoff" : "Prepare shared handoff"}
            </Button>
            {handoff ? (
              <Button disabled={busy} onClick={() => void runReview()} size="sm" type="button">
                <ShieldCheckIcon /> Run read-only {agentLabel(reviewerAdapter)} review
              </Button>
            ) : null}
          </div>
          {error ? <Alert variant="destructive"><AlertDescription>{error}</AlertDescription></Alert> : null}
          <ScrollArea className="max-h-[68vh] pr-3">
            <div className="grid gap-4">
              {handoff ? <HandoffPreview handoff={handoff} /> : null}
              {reviews.map((review) => (
                <ReviewCard
                  busy={busy}
                  implementationAdapter={implementationAdapter}
                  key={review.id}
                  onDismiss={(finding) => void setFindingStatus(finding, "dismissed")}
                  onResolve={(finding) => void setFindingStatus(finding, "resolved")}
                  onSend={() => void sendFollowup(review)}
                  review={review}
                />
              ))}
            </div>
          </ScrollArea>
        </DialogContent>
      </Dialog>
    </>
  );
}

function HandoffPreview({ handoff }: { handoff: ReviewHandoff }) {
  return (
    <section className="grid gap-3 rounded-lg border bg-muted/15 p-3" aria-label="Shared review handoff">
      <div className="flex flex-wrap items-center gap-2 text-[10px] text-muted-foreground">
        <Badge variant="secondary">{handoff.totalBytes.toLocaleString()} / {handoff.maxBytes.toLocaleString()} bytes</Badge>
        <span>{handoff.contextPack.items.length} context items</span>
        <span>{handoff.implementationEvents.length} normalized events</span>
        <code className="ml-auto">sha256:{handoff.diffSha256.slice(0, 12)}</code>
      </div>
      <div className="grid gap-1">
        <p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">Implementation evidence</p>
        {handoff.implementationEvents.map((event) => (
          <p className="truncate text-xs" key={event.sequence}>
            <span className="text-muted-foreground">{event.eventType} · </span>{event.summary}
          </p>
        ))}
      </div>
      <details>
        <summary className="cursor-pointer text-xs font-medium">Exact diff sent to reviewer</summary>
        <pre className="mt-2 max-h-72 overflow-auto rounded-md border bg-background p-3 text-[10px] leading-relaxed">{handoff.diff}</pre>
      </details>
    </section>
  );
}

interface ReviewCardProps {
  busy: boolean;
  implementationAdapter: AdapterKind;
  onDismiss: (finding: ReviewFinding) => void;
  onResolve: (finding: ReviewFinding) => void;
  onSend: () => void;
  review: ReviewRecord;
}

function ReviewCard({ busy, implementationAdapter, onDismiss, onResolve, onSend, review }: ReviewCardProps) {
  const openFindings = review.findings.filter((finding) => finding.status === "open");
  return (
    <section className="grid gap-3 rounded-lg border p-3" aria-label={`${review.reviewerAdapter} review`}>
      <div className="flex items-center gap-2">
        {review.verdict === "pass" ? <CheckCircle2Icon className="size-4 text-emerald-600" /> : <TriangleAlertIcon className="size-4 text-amber-600" />}
        <Badge variant={review.verdict === "pass" ? "secondary" : "outline"}>{review.verdict.replaceAll("_", " ")}</Badge>
        <span className="text-xs text-muted-foreground">{agentLabel(review.reviewerAdapter)}</span>
      </div>
      <p className="text-xs">{review.summary}</p>
      {review.findings.map((finding) => (
        <article className="grid gap-2 rounded-md border bg-muted/15 p-2" key={finding.id}>
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant={finding.severity === "blocker" || finding.severity === "error" ? "destructive" : "outline"}>{finding.severity}</Badge>
            <span className="text-xs font-medium">{finding.title}</span>
            <Badge className="ml-auto" variant="secondary">{finding.status}</Badge>
          </div>
          <p className="text-xs text-muted-foreground">{finding.body}</p>
          <code className="text-[10px]">{finding.relativePath}:{finding.line} · {finding.evidence.join(", ")}</code>
          {finding.status === "open" ? (
            <div className="flex gap-2">
              <Button disabled={busy} onClick={() => onResolve(finding)} size="xs" type="button" variant="outline">Resolve</Button>
              <Button disabled={busy} onClick={() => onDismiss(finding)} size="xs" type="button" variant="ghost">Dismiss</Button>
            </div>
          ) : null}
        </article>
      ))}
      {openFindings.length > 0 ? (
        <Button disabled={busy} onClick={onSend} size="sm" type="button">
          <SendIcon /> Send {openFindings.length} finding{openFindings.length === 1 ? "" : "s"} to {agentLabel(implementationAdapter)}
          <ArrowRightIcon />
        </Button>
      ) : null}
    </section>
  );
}

function agentLabel(adapter: AdapterKind) {
  return adapter === "claude" ? "Claude Code" : "Codex";
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}
