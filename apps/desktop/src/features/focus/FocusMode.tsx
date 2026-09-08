import { ArrowLeftIcon, TerminalIcon } from "lucide-react";
import type { ReactNode } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { KeyHint, MetaLabel, Mono, StateDot } from "@/shell/primitives";

export interface FocusModeProps {
  explorer: ReactNode;
  editor: ReactNode;
  terminal: ReactNode;
  terminalOpen: boolean;
  onToggleTerminal: () => void;
  /** The one way back. Always visible, always the same place. */
  onReturn: () => void;
  returnLabel: string;
  /** What the conversation is doing while it is not on screen. */
  conversationState?: ActivityState;
  /** Unread activity since focus mode was entered. */
  unread?: number;
  openPath?: string | null;
  branchName?: string | null;
  returnHint?: string;
}

/**
 * The editor, the explorer and the terminal, full height — and borrowed.
 *
 * Mars is a conversation, and this mode suspends that claim rather than
 * revoking it. Everything about the frame says so: the bar at the top is not
 * chrome to be ignored but the way back, it names what it is returning to, and
 * it keeps reporting what the conversation is doing while nobody is watching
 * it. Nothing is hidden behind a menu, and the return is one click from
 * anywhere on the screen.
 *
 * The other half of "borrowed" is not visual: a file saved here is written into
 * the task's activity record, so work done by hand lands in the same transcript
 * as work done by an agent and the record stays the whole truth.
 */
export function FocusMode({
  branchName,
  conversationState = "idle",
  editor,
  explorer,
  onReturn,
  onToggleTerminal,
  openPath,
  returnHint,
  returnLabel,
  terminal,
  terminalOpen,
  unread = 0,
}: FocusModeProps) {
  return (
    <div className="flex h-screen min-w-[820px] flex-col overflow-hidden bg-sunken text-foreground">
      <header
        aria-label="Focus mode"
        className="flex h-[var(--shell-header-height)] shrink-0 items-center gap-2 border-b border-border bg-surface px-2"
      >
        <button
          type="button"
          onClick={onReturn}
          className="flex items-center gap-1.5 border border-border px-2 py-0.5 text-[11px] text-foreground outline-none hover:border-border-strong focus-visible:border-border-strong"
        >
          <ArrowLeftIcon className="size-3.5" />
          Back to {returnLabel}
          {returnHint ? (
            <KeyHint className="border-transparent text-current opacity-70" keys={returnHint} />
          ) : null}
        </button>

        {unread > 0 || conversationState !== "idle" ? (
          <span className="flex items-center gap-1.5">
            <StateDot state={conversationState} />
            <span className="text-[11px] text-muted-foreground">
              {unread > 0 ? `${unread} new in the conversation` : "the conversation is live"}
            </span>
          </span>
        ) : null}

        <MetaLabel className="ml-auto">Focus</MetaLabel>
        {openPath ? <Mono className="max-w-80 truncate text-faint">{openPath}</Mono> : null}
        {branchName ? <Mono className="text-faint">{branchName}</Mono> : null}

        <button
          type="button"
          aria-pressed={terminalOpen}
          onClick={onToggleTerminal}
          className={cn(
            "flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong",
            terminalOpen ? "text-foreground" : "text-muted-foreground",
          )}
        >
          <TerminalIcon className="size-3" />
          Terminal
        </button>
      </header>

      <div className="flex min-h-0 flex-1">
        <aside
          aria-label="Files"
          className="flex w-64 shrink-0 flex-col border-r border-border bg-surface"
        >
          {explorer}
        </aside>

        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          <div className="min-h-0 flex-1 overflow-hidden bg-background">{editor}</div>
          {terminalOpen ? (
            <div
              aria-label="Terminal"
              className="h-64 shrink-0 overflow-hidden border-t border-border bg-background"
            >
              {terminal}
            </div>
          ) : null}
        </div>
      </div>

      <footer className="flex h-[var(--shell-status-height)] shrink-0 items-center gap-3 border-t border-border bg-surface px-2 text-[11px]">
        <span className="text-faint">
          Borrowed from the conversation. Every save here is recorded in the task's activity.
        </span>
        <button
          type="button"
          onClick={onReturn}
          className="ml-auto text-muted-foreground underline-offset-2 outline-none hover:text-foreground hover:underline focus-visible:text-foreground"
        >
          Return to {returnLabel}
        </button>
      </footer>
    </div>
  );
}
