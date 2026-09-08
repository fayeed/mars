import { AtSignIcon, PaperclipIcon, SendIcon, ShieldIcon, SlashIcon, XIcon } from "lucide-react";
import {
  useCallback,
  useId,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type KeyboardEvent,
} from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { applyCompletion, filterCandidates, readTrigger } from "@/shell/composer-input";
import { chordHint } from "@/shell/keymap";
import type {
  ComposerAgent,
  ComposerAttachment,
  ComposerSkill,
  ComposerSubmission,
  MentionTarget,
} from "@/shell/model";
import { KeyHint, Mono, StateDot } from "@/shell/primitives";
import { useShell } from "@/shell/shell-store";

export interface ComposerProps {
  agents?: ComposerAgent[];
  attachments?: ComposerAttachment[];
  mentions?: MentionTarget[];
  skills?: ComposerSkill[];
  selectedAgentId?: string;
  onSelectAgent?: (agentId: string) => void;
  onAttach?: () => void;
  onRemoveAttachment?: (attachmentId: string) => void;
  onSubmit?: (submission: ComposerSubmission) => void;
  onStop?: () => void;
  /** `streaming` shows Stop; `blocked` says why sending will not help yet. */
  state?: ActivityState;
  blockedReason?: string;
  placeholder?: string;
}

interface MenuItem {
  id: string;
  token: string;
  primary: string;
  secondary: string;
}

/**
 * The composer stays alive and usable while diffs, editors and panels are
 * open — it belongs to the conversation column, not to any screen. Before you
 * send, it states which agent will act and what that agent may already do
 * without stopping to ask.
 */
export function Composer({
  agents = [],
  attachments = [],
  blockedReason,
  mentions = [],
  onAttach,
  onRemoveAttachment,
  onSelectAgent,
  onStop,
  onSubmit,
  placeholder = "Describe the change. @ for files and symbols, / for skills.",
  selectedAgentId,
  skills = [],
  state = "idle",
}: ComposerProps) {
  const { registerRegion } = useShell();
  const listboxId = useId();
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const [value, setValue] = useState("");
  const [caret, setCaret] = useState(0);
  const [menuDismissed, setMenuDismissed] = useState(false);
  const [highlight, setHighlight] = useState(0);

  const agent = agents.find((candidate) => candidate.id === selectedAgentId) ?? agents[0];
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

  const canSend = value.trim().length > 0 && Boolean(agent) && state !== "blocked";

  return (
    <div
      data-slot="composer"
      className="shrink-0 border-t border-border bg-surface"
      aria-label="Composer"
    >
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

      <div className="relative">
        {menuOpen ? (
          <ul
            id={listboxId}
            role="listbox"
            aria-label={trigger?.kind === "skill" ? "Skills" : "Files and symbols"}
            className="absolute bottom-full left-0 z-20 mb-px max-h-64 w-full overflow-y-auto border border-border bg-popover"
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

        <label className="ml-auto flex items-center gap-1.5 text-[11px] text-muted-foreground">
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
        ) : (
          <button
            type="button"
            disabled={!canSend}
            onClick={submit}
            className="flex items-center gap-1.5 border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:border-border disabled:bg-transparent disabled:text-faint"
          >
            <SendIcon className="size-3.5" />
            Send
            <KeyHint
              className="border-transparent text-current opacity-70"
              keys={chordHint({ key: "Enter", mod: true })}
            />
          </button>
        )}
      </div>

      <div
        className="flex items-center gap-1.5 border-t border-border px-2 py-1"
        aria-live="polite"
        data-slot="permission-hint"
      >
        <ShieldIcon className="size-3 shrink-0 text-faint" />
        {state === "blocked" ? (
          <span className="text-[11px] text-primary">
            {blockedReason ?? "A permission request is waiting for your answer."}
          </span>
        ) : agent ? (
          <span className="text-[11px] text-muted-foreground">
            <span className="text-foreground">{agent.name}</span> may {agent.permissionScope}
            {agent.asks && agent.asks.length > 0 ? (
              <>
                {" · asks before "}
                <Mono className="text-faint">{agent.asks.join(", ")}</Mono>
              </>
            ) : null}
          </span>
        ) : (
          <span className="text-[11px] text-faint">No agent selected — nothing can run yet.</span>
        )}
      </div>
    </div>
  );
}
