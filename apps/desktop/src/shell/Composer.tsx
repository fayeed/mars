import {
  AtSignIcon,
  FileTextIcon,
  PaperclipIcon,
  SendIcon,
  ShieldIcon,
  SlashIcon,
  XIcon,
} from "lucide-react";
import {
  useCallback,
  useId,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type DragEvent,
  type KeyboardEvent,
} from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { applyCompletion, filterCandidates, readTrigger } from "@/shell/composer-input";
import { chordHint } from "@/shell/keymap";
import type {
  ComposerAgent,
  ComposerAttachment,
  ComposerScope,
  ComposerSkill,
  ComposerSubmission,
  MentionTarget,
} from "@/shell/model";
import { KeyHint, MetaLabel, Mono, StateDot } from "@/shell/primitives";
import { useShell } from "@/shell/shell-store";

export interface ComposerTarget {
  mode: "task" | "new";
  /** The open task's title, when there is one to continue. */
  taskTitle?: string;
}

export interface ComposerProps {
  agents?: ComposerAgent[];
  attachments?: ComposerAttachment[];
  mentions?: MentionTarget[];
  skills?: ComposerSkill[];
  scopes?: ComposerScope[];
  selectedAgentId?: string;
  selectedScopeId?: string;
  onSelectAgent?: (agentId: string) => void;
  onSelectScope?: (scopeId: string) => void;
  onAttach?: () => void;
  /** Files dropped onto the composer, in the order the browser reported them. */
  onDropFiles?: (files: File[]) => void;
  onRemoveAttachment?: (attachmentId: string) => void;
  onSubmit?: (submission: ComposerSubmission) => void;
  onStop?: () => void;
  /** Controlled draft. Omit to let the composer own its own text. */
  value?: string;
  onValueChange?: (value: string) => void;
  /**
   * `streaming` adds Stop beside Send, and a message sent then waits for the
   * run to finish; `blocked` says why sending will not help yet.
   */
  state?: ActivityState;
  /**
   * Where the message will go: the open task's conversation, or a new task.
   * Said in the composer, because the same words do very different things.
   */
  target?: ComposerTarget;
  onTargetChange?: (mode: ComposerTarget["mode"]) => void;
  /** Messages already waiting for the running agent to finish. */
  queuedCount?: number;
  blockedReason?: string;
  placeholder?: string;
  /** Live `@` search. Omit to filter `mentions` in the browser. */
  onSearchMentions?: (query: string) => void;
}

interface MenuItem {
  id: string;
  token: string;
  primary: string;
  secondary: string;
  /** Present for skills: the instruction text picking this would hand over. */
  instructions?: string;
}

/**
 * The composer stays alive and usable while diffs, editors and panels are
 * open — it belongs to the conversation column, not to any screen. Before you
 * send, it states which agent will act, where that agent may work, and what it
 * may already do without stopping to ask.
 */
export function Composer({
  agents = [],
  attachments = [],
  blockedReason,
  mentions = [],
  onAttach,
  onDropFiles,
  onRemoveAttachment,
  onSearchMentions,
  onSelectAgent,
  onSelectScope,
  onStop,
  onSubmit,
  onValueChange,
  placeholder = "Describe the change. @ for files and symbols, / for skills.",
  scopes = [],
  selectedAgentId,
  selectedScopeId,
  skills = [],
  state = "idle",
  target,
  onTargetChange,
  queuedCount = 0,
  value: controlledValue,
}: ComposerProps) {
  const { registerRegion } = useShell();
  const listboxId = useId();
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const [internalValue, setInternalValue] = useState("");
  const [caret, setCaret] = useState(0);
  const [menuDismissed, setMenuDismissed] = useState(false);
  const [highlight, setHighlight] = useState(0);
  const [dropping, setDropping] = useState(false);

  const value = controlledValue ?? internalValue;
  const setValue = useCallback(
    (next: string) => {
      if (controlledValue === undefined) setInternalValue(next);
      onValueChange?.(next);
    },
    [controlledValue, onValueChange],
  );

  const agent = agents.find((candidate) => candidate.id === selectedAgentId) ?? agents[0];
  const scope = scopes.find((candidate) => candidate.id === selectedScopeId) ?? scopes[0];
  const trigger = menuDismissed ? null : readTrigger(value, caret);

  const items = useMemo<MenuItem[]>(() => {
    if (!trigger) return [];
    if (trigger.kind === "mention") {
      return filterCandidates(mentions, trigger.query).map((target) => ({
        id: target.id,
        token: `@${target.detail}`,
        primary: target.label,
        secondary: target.detail,
      }));
    }
    return filterCandidates(skills, trigger.query).map((skill) => ({
      id: skill.id,
      token: `/${skill.name}`,
      primary: skill.name,
      secondary: skill.description,
      instructions: skill.instructions,
    }));
  }, [mentions, skills, trigger]);

  const menuOpen = Boolean(trigger) && items.length > 0;
  const activeItem = menuOpen ? items[Math.min(highlight, items.length - 1)] : undefined;

  const setRegion = useCallback(
    (element: HTMLTextAreaElement | null) => {
      textareaRef.current = element;
      registerRegion("composer", element);
      return () => registerRegion("composer", null);
    },
    [registerRegion],
  );

  function syncCaret(element: HTMLTextAreaElement) {
    setCaret(element.selectionStart ?? element.value.length);
  }

  function onChange(event: ChangeEvent<HTMLTextAreaElement>) {
    setValue(event.target.value);
    setMenuDismissed(false);
    setHighlight(0);
    syncCaret(event.target);
    // The index knows more files than any one page of them, so the screen is
    // told what is being typed and may widen the candidate list.
    const next = readTrigger(event.target.value, event.target.selectionStart ?? 0);
    if (next?.kind === "mention") onSearchMentions?.(next.query);
  }

  function complete(item: MenuItem) {
    if (!trigger) return;
    const result = applyCompletion(value, trigger, item.token);
    setValue(result.value);
    setCaret(result.caret);
    setMenuDismissed(true);
    const element = textareaRef.current;
    if (element) {
      element.value = result.value;
      element.setSelectionRange(result.caret, result.caret);
      element.focus();
    }
  }

  function submit() {
    const text = value.trim();
    if (!text || !agent) return;
    onSubmit?.({
      text,
      agentId: agent.id,
      attachmentIds: attachments.map((attachment) => attachment.id),
      scopeId: scope?.id,
    });
    setValue("");
    setCaret(0);
    setMenuDismissed(false);
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (menuOpen) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const step = event.key === "ArrowDown" ? 1 : -1;
        setHighlight((current) => (current + step + items.length) % items.length);
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        if (activeItem) complete(activeItem);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setMenuDismissed(true);
        return;
      }
    }

    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      submit();
    }
  }

  function onDrop(event: DragEvent<HTMLDivElement>) {
    if (!onDropFiles) return;
    event.preventDefault();
    setDropping(false);
    const files = Array.from(event.dataTransfer.files);
    if (files.length > 0) onDropFiles(files);
  }

  // An unavailable agent does not disable the composer: `/ask` is a question
  // about the repository and needs no agent at all, and refusing to let it be
  // typed would take away the one thing that still works.
  const canSend = value.trim().length > 0 && Boolean(agent) && state !== "blocked";

  return (
    <div
      data-slot="composer"
      className={cn(
        "relative shrink-0 border-t border-border bg-surface",
        dropping && "shadow-[inset_0_0_0_1px_var(--ring)]",
      )}
      aria-label="Composer"
      onDragOver={(event) => {
        if (!onDropFiles) return;
        event.preventDefault();
        setDropping(true);
      }}
      onDragLeave={(event) => {
        // Only the composer leaving counts; crossing a child element does not.
        if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
        setDropping(false);
      }}
      onDrop={onDrop}
    >
      {dropping ? (
        <div
          className="pointer-events-none absolute inset-0 z-30 flex items-center justify-center bg-surface/90"
          aria-hidden="true"
        >
          <span className="text-[12px] text-foreground">
            Drop to attach. Files are captured, classified, and shown before they reach an agent.
          </span>
        </div>
      ) : null}

      {attachments.length > 0 ? (
        <ul
          className="flex flex-wrap gap-1 border-b border-border px-2 py-1.5"
          aria-label="Attachments"
        >
          {attachments.map((attachment) => (
            <li
              key={attachment.id}
              className="flex items-center gap-1 border border-border px-1.5 py-0.5"
            >
              <Mono className="text-muted-foreground">{attachment.label}</Mono>
              {attachment.detail ? <Mono className="text-faint">{attachment.detail}</Mono> : null}
              <button
                type="button"
                className="text-faint outline-none hover:text-destructive focus-visible:text-destructive"
                onClick={() => onRemoveAttachment?.(attachment.id)}
              >
                <XIcon className="size-3" />
                <span className="sr-only">Remove {attachment.label}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}

      {target ? (
        <div
          className="flex items-center gap-2 border-b border-border px-2.5 py-1 text-[11px]"
          data-slot="composer-target"
        >
          {target.mode === "task" && target.taskTitle ? (
            <>
              <span className="text-faint">Continuing</span>
              <span className="min-w-0 truncate text-foreground">{target.taskTitle}</span>
              {queuedCount > 0 ? (
                <Mono className="shrink-0 text-primary">{queuedCount} queued</Mono>
              ) : null}
              <button
                type="button"
                className="ml-auto shrink-0 text-muted-foreground outline-none hover:text-foreground focus-visible:text-foreground"
                onClick={() => onTargetChange?.("new")}
              >
                New task instead
              </button>
            </>
          ) : (
            <>
              <span className="text-faint">New task</span>
              <span className="text-muted-foreground">— planned in its own branch and worktree</span>
              {target.taskTitle ? (
                <button
                  type="button"
                  className="ml-auto min-w-0 truncate text-muted-foreground outline-none hover:text-foreground focus-visible:text-foreground"
                  onClick={() => onTargetChange?.("task")}
                >
                  Continue {target.taskTitle} instead
                </button>
              ) : null}
            </>
          )}
        </div>
      ) : null}

      <div className="relative">
        {menuOpen ? (
          <div className="absolute bottom-full left-0 z-20 mb-px flex w-full border border-border bg-popover">
            <ul
              id={listboxId}
              role="listbox"
              aria-label={trigger?.kind === "skill" ? "Skills" : "Files and symbols"}
              className={cn(
                "max-h-64 overflow-y-auto",
                activeItem?.instructions ? "w-1/2 shrink-0 border-r border-border" : "w-full",
              )}
            >
              {items.map((item, index) => (
                <li
                  key={item.id}
                  id={`${listboxId}-${item.id}`}
                  role="option"
                  aria-selected={index === highlight}
                  onMouseDown={(event) => {
                    event.preventDefault();
                    complete(item);
                  }}
                  onMouseEnter={() => setHighlight(index)}
                  className={cn(
                    "flex cursor-pointer items-baseline gap-2 px-2 py-1",
                    index === highlight && "bg-surface-raised",
                  )}
                >
                  <span className="text-[12px] text-foreground">{item.primary}</span>
                  <Mono className="truncate text-faint">{item.secondary}</Mono>
                </li>
              ))}
            </ul>

            {activeItem?.instructions ? (
              // Picking a skill hands an agent instructions somebody else wrote.
              // They are shown verbatim, before the choice, not summarised after.
              <div className="flex min-w-0 flex-1 flex-col gap-1 p-2">
                <MetaLabel className="flex items-center gap-1.5">
                  <FileTextIcon className="size-3" />
                  {activeItem.primary} instructions
                </MetaLabel>
                <pre className="max-h-56 overflow-auto whitespace-pre-wrap font-mono text-[11px] leading-relaxed text-muted-foreground">
                  {activeItem.instructions}
                </pre>
              </div>
            ) : null}
          </div>
        ) : null}

        <textarea
          ref={setRegion}
          aria-label="Message"
          role="combobox"
          aria-expanded={menuOpen}
          aria-controls={menuOpen ? listboxId : undefined}
          aria-activedescendant={activeItem ? `${listboxId}-${activeItem.id}` : undefined}
          autoComplete="off"
          className="max-h-56 min-h-16 w-full resize-none bg-transparent px-2.5 py-2 text-[13px] leading-relaxed text-foreground outline-none placeholder:text-faint"
          onChange={onChange}
          onClick={(event) => syncCaret(event.currentTarget)}
          onKeyUp={(event) => syncCaret(event.currentTarget)}
          onKeyDown={onKeyDown}
          placeholder={placeholder}
          rows={3}
          value={value}
        />
      </div>

      <div className="flex flex-wrap items-center gap-2 border-t border-border px-2 py-1.5">
        <button
          type="button"
          className="flex items-center gap-1 text-[11px] text-muted-foreground outline-none hover:text-foreground focus-visible:text-foreground"
          onClick={onAttach}
        >
          <PaperclipIcon className="size-3.5" />
          Attach
        </button>
        <span className="flex items-center gap-1 text-[11px] text-faint">
          <AtSignIcon className="size-3.5" />
          files
        </span>
        <span className="flex items-center gap-1 text-[11px] text-faint">
          <SlashIcon className="size-3.5" />
          skills
        </span>

        {scopes.length > 0 ? (
          <label className="ml-auto flex items-center gap-1.5 text-[11px] text-muted-foreground">
            <span className="sr-only">Scope</span>
            <select
              className="border border-border bg-transparent px-1.5 py-0.5 text-[11px] text-foreground outline-none focus-visible:border-border-strong"
              value={scope?.id ?? ""}
              onChange={(event) => onSelectScope?.(event.target.value)}
              aria-label="Scope"
            >
              {scopes.map((candidate) => (
                <option key={candidate.id} value={candidate.id}>
                  {candidate.label}
                </option>
              ))}
            </select>
          </label>
        ) : null}

        <label
          className={cn(
            "flex items-center gap-1.5 text-[11px] text-muted-foreground",
            scopes.length === 0 && "ml-auto",
          )}
        >
          <span className="sr-only">Agent</span>
          <select
            className="border border-border bg-transparent px-1.5 py-0.5 text-[11px] text-foreground outline-none focus-visible:border-border-strong"
            value={agent?.id ?? ""}
            onChange={(event) => onSelectAgent?.(event.target.value)}
            aria-label="Agent"
          >
            {agents.length === 0 ? <option value="">No agent configured</option> : null}
            {agents.map((candidate) => (
              <option key={candidate.id} value={candidate.id}>
                {candidate.name}
              </option>
            ))}
          </select>
        </label>

        {state === "streaming" ? (
          <button
            type="button"
            className="flex items-center gap-1.5 border border-border px-2 py-1 text-[11px] text-foreground outline-none hover:border-border-strong"
            onClick={onStop}
          >
            <StateDot state="streaming" />
            Stop
          </button>
        ) : null}
        <button
          type="button"
          disabled={!canSend}
          onClick={submit}
          className="flex items-center gap-1.5 border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:border-border disabled:bg-transparent disabled:text-faint"
        >
          <SendIcon className="size-3.5" />
          {state === "streaming" && target?.mode === "task" ? "Queue" : "Send"}
          <KeyHint
            className="border-transparent text-current opacity-70"
            keys={chordHint({ key: "Enter", mod: true })}
          />
        </button>
      </div>

      <div
        className="flex flex-wrap items-center gap-x-1.5 gap-y-1 border-t border-border px-2 py-1"
        aria-live="polite"
        data-slot="permission-hint"
      >
        <ShieldIcon className="size-3 shrink-0 text-faint" />
        {state === "blocked" ? (
          <span className="text-[11px] text-primary">
            {blockedReason ?? "A permission request is waiting for your answer."}
          </span>
        ) : agent?.unavailable ? (
          // Said before the message is written, not after it is sent. Mars runs
          // the CLIs installed on this machine and bundles none of them, so an
          // absent agent is the normal case on a fresh install rather than an
          // error to be discovered at launch time.
          <span className="flex items-center gap-1.5 text-[11px] text-primary">
            <StateDot state="blocked" />
            <span>
              <span className="text-foreground">{agent.name}</span> {agent.unavailable}
            </span>
          </span>
        ) : agent ? (
          <>
            <span className="text-[11px] text-muted-foreground">
              <span className="text-foreground">{agent.name}</span> may {agent.permissionScope}
              {agent.asks && agent.asks.length > 0 ? (
                <>
                  {" · asks before "}
                  <Mono className="text-faint">{agent.asks.join(", ")}</Mono>
                </>
              ) : null}
            </span>
            {scope ? (
              <span className="text-[11px] text-muted-foreground">
                {" · within "}
                <Mono className="text-faint">{scope.globs.join(" ") || scope.label}</Mono>
              </span>
            ) : null}
          </>
        ) : (
          <span className="text-[11px] text-faint">No agent selected — nothing can run yet.</span>
        )}
      </div>
    </div>
  );
}
