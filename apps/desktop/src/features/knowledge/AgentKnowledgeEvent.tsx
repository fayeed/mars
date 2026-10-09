import type { ActivityEvent } from "@mars/shared-types";
import { ArrowUpRightIcon, BookmarkIcon, ScaleIcon } from "lucide-react";

import { MetaLabel, Mono } from "@/shell/primitives";

/** The Mars MCP tools that write knowledge for a person to review. */
export const KNOWLEDGE_TOOLS = ["mcp__mars__record_decision", "mcp__mars__remember"] as const;

function text(input: unknown, name: string): string | null {
  if (typeof input !== "object" || input === null) return null;
  const value = (input as Record<string, unknown>)[name];
  return typeof value === "string" && value.trim() ? value : null;
}

function list(input: unknown, name: string): string[] {
  if (typeof input !== "object" || input === null) return [];
  const value = (input as Record<string, unknown>)[name];
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

/** Whether a record row is an agent writing knowledge, and so gets a card. */
export function isKnowledgeEvent(event: ActivityEvent): boolean {
  if (event.kind !== "tool_call") return false;
  const name = text(event.payload, "name");
  return name !== null && (KNOWLEDGE_TOOLS as readonly string[]).includes(name);
}

/**
 * An agent recording a decision or proposing a memory, as it reads in the
 * conversation: what it said, and where to settle it. Neither is in effect
 * until a person approves it, and the card says so.
 */
export function AgentKnowledgeEvent({
  event,
  onReview,
}: {
  event: ActivityEvent;
  onReview: (tab: "decisions" | "memory") => void;
}) {
  const payload = event.payload as { name?: string; input?: unknown } | null;
  const input = payload?.input;
  const decision = payload?.name === "mcp__mars__record_decision";
  const files = list(input, "files");
  const alternatives = list(input, "alternatives");

  return (
    <div
      className="flex flex-col gap-1 px-3 py-2"
      data-slot="agent-knowledge"
      aria-label={decision ? "Decision recorded" : "Memory proposed"}
    >
      <div className="flex items-center gap-1.5">
        {decision ? (
          <ScaleIcon className="size-3.5 text-primary" />
        ) : (
          <BookmarkIcon className="size-3.5 text-primary" />
        )}
        <MetaLabel>{decision ? "Decision recorded" : "Memory proposed"}</MetaLabel>
        <Mono className="text-faint">waiting on your review</Mono>
        <button
          type="button"
          onClick={() => onReview(decision ? "decisions" : "memory")}
          className="ml-auto flex items-center gap-1 text-[11px] text-muted-foreground outline-none hover:text-foreground focus-visible:text-foreground"
        >
          Review
          <ArrowUpRightIcon className="size-3" />
        </button>
      </div>
      {decision ? (
        <>
          <p className="text-[13px] font-medium text-foreground">{text(input, "title") ?? "Untitled"}</p>
          <p className="whitespace-pre-wrap text-[12px] leading-relaxed text-foreground">
            {text(input, "decision")}
          </p>
          {text(input, "rationale") ? (
            <p className="whitespace-pre-wrap text-[12px] leading-relaxed text-muted-foreground">
              Why: {text(input, "rationale")}
            </p>
          ) : null}
          {alternatives.length > 0 ? (
            <Mono className="text-faint">not chosen: {alternatives.join(" · ")}</Mono>
          ) : null}
          {files.length > 0 ? <Mono className="truncate text-faint">{files.join(", ")}</Mono> : null}
        </>
      ) : (
        <>
          <p className="text-[12px] leading-relaxed text-foreground">{text(input, "text")}</p>
          <Mono className="text-faint">
            {[text(input, "kind"), text(input, "scope") ?? "repo"].filter(Boolean).join(" · ")}
          </Mono>
        </>
      )}
    </div>
  );
}
