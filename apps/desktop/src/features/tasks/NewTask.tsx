import type { StoredIssue } from "@mars/shared-types";
import { DownloadIcon, PencilLineIcon } from "lucide-react";
import { useState } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import type { ComposerAgent, ComposerScope } from "@/shell/model";
import { MetaLabel, Mono, StateLabel } from "@/shell/primitives";

export interface NewTaskProps {
  agents: ComposerAgent[];
  scopes: ComposerScope[];
  selectedAgentId: string;
  selectedScopeId: string;
  onSelectAgent: (id: string) => void;
  onSelectScope: (id: string) => void;
  /** Describe-it: the text becomes the task's intent. */
  onDescribe: (text: string) => void;
  /** Import: fetches the issue and pre-fills the description. */
  onImportIssue: (remoteName: string, issueNumber: number) => void;
  importedIssue: StoredIssue | null;
  importState: ActivityState;
  error?: string | null;
  busy?: boolean;
}

/**
 * The only two ways work enters Mars.
 *
 * Both end in the same place — a description an agent can act on — and the
 * screen says so by putting them side by side rather than hiding the import
 * behind a menu. An imported issue keeps its link: the task knows what it came
 * from, and the description stays editable, because an issue is what somebody
 * asked for and not necessarily what should be built.
 */
export function NewTask({
  agents,
  busy = false,
  error,
  importState,
  importedIssue,
  onDescribe,
  onImportIssue,
  onSelectAgent,
  onSelectScope,
  scopes,
  selectedAgentId,
  selectedScopeId,
}: NewTaskProps) {
  const [mode, setMode] = useState<"describe" | "import">("describe");
  const [text, setText] = useState("");
  const [remote, setRemote] = useState("origin");
  const [issueNumber, setIssueNumber] = useState("");

  // An imported issue is a starting point for the description, not a
  // replacement for it.
  const description =
    importedIssue && mode === "import" && text.length === 0
      ? `${importedIssue.title}\n\n${importedIssue.body}`
      : text;

  return (
    <section aria-label="New task" className="flex flex-col gap-3 px-3 py-3">
      <div className="flex items-center gap-1" role="tablist" aria-label="How work enters Mars">
        {(["describe", "import"] as const).map((candidate) => (
          <button
            key={candidate}
            type="button"
            role="tab"
            aria-selected={mode === candidate}
            onClick={() => setMode(candidate)}
            className={cn(
              "flex items-center gap-1.5 border px-2 py-1 text-[11px] outline-none",
              mode === candidate
                ? "border-primary text-foreground"
                : "border-border text-muted-foreground hover:border-border-strong",
            )}
          >
            {candidate === "describe" ? (
              <PencilLineIcon className="size-3" />
            ) : (
              <DownloadIcon className="size-3" />
            )}
            {candidate === "describe" ? "Describe it" : "Import an issue"}
          </button>
        ))}
      </div>

      {mode === "import" ? (
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center gap-1">
            <input
              aria-label="Remote"
              className="w-24 shrink-0 border border-border bg-transparent px-1.5 py-0.5 font-mono text-[11px] text-foreground outline-none focus-visible:border-border-strong"
              onChange={(event) => setRemote(event.target.value)}
              value={remote}
            />
            <input
              aria-label="Issue number"
              className="w-24 shrink-0 border border-border bg-transparent px-1.5 py-0.5 font-mono text-[11px] text-foreground outline-none focus-visible:border-border-strong"
              inputMode="numeric"
              onChange={(event) => setIssueNumber(event.target.value.replace(/\D/g, ""))}
              placeholder="123"
              value={issueNumber}
            />
            <button
              type="button"
              disabled={busy || issueNumber.length === 0}
              onClick={() => onImportIssue(remote, Number(issueNumber))}
              className="border border-border px-2 py-0.5 text-[11px] outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
            >
              Fetch
            </button>
            {importState === "loading" ? <StateLabel state="loading" detail="reading" /> : null}
          </div>

          {importedIssue ? (
            <div className="flex flex-col gap-0.5 border border-border px-2 py-1">
              <div className="flex flex-wrap items-baseline gap-2">
                <Mono className="text-faint">#{importedIssue.number}</Mono>
                <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">
                  {importedIssue.title}
                </span>
                <Mono className="text-faint">{importedIssue.state}</Mono>
              </div>
              <Mono className="break-all text-faint">{importedIssue.url}</Mono>
              <span className="text-[11px] text-faint">
                The task keeps this link. Editing the description below does not change the issue.
              </span>
            </div>
          ) : null}
        </div>
      ) : null}

      <div className="flex flex-col gap-1.5">
        <MetaLabel>What should happen</MetaLabel>
        <textarea
          aria-label="Describe the task"
          className="min-h-28 w-full resize-y border border-border bg-transparent px-1.5 py-1 text-[12px] leading-relaxed text-foreground outline-none focus-visible:border-border-strong"
          onChange={(event) => setText(event.target.value)}
          placeholder="Describe the change. This becomes the task's intent and the agent's assignment."
          value={description}
        />
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <label className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <span className="sr-only">Agent</span>
          <select
            aria-label="Agent"
            className="border border-border bg-transparent px-1.5 py-0.5 text-[11px] text-foreground outline-none focus-visible:border-border-strong"
            onChange={(event) => onSelectAgent(event.target.value)}
            value={selectedAgentId}
          >
            {agents.map((agent) => (
              <option key={agent.id} value={agent.id}>
                {agent.name}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <span className="sr-only">Scope</span>
          <select
            aria-label="Scope"
            className="border border-border bg-transparent px-1.5 py-0.5 text-[11px] text-foreground outline-none focus-visible:border-border-strong"
            onChange={(event) => onSelectScope(event.target.value)}
            value={selectedScopeId}
          >
            {scopes.map((scope) => (
              <option key={scope.id} value={scope.id}>
                {scope.label}
              </option>
            ))}
          </select>
        </label>

        <button
          type="button"
          disabled={busy || description.trim().length === 0}
          onClick={() => onDescribe(description.trim())}
          className="ml-auto border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:border-border disabled:bg-transparent disabled:text-faint"
        >
          {busy ? "Planning…" : "Plan this task"}
        </button>
      </div>

      {error ? (
        <p className="border border-destructive/40 px-2 py-1 text-[11px] text-destructive">
          <Mono>{error}</Mono>
        </p>
      ) : null}

      <p className="text-[11px] text-faint">
        Nothing is created yet. The next screen is the worktree plan, and nothing touches Git until
        you approve it.
      </p>
    </section>
  );
}
