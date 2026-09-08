import type { SkillInstallation, SkillScope } from "@mars/shared-types";
import { PowerIcon, PowerOffIcon } from "lucide-react";
import { useState } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateBlock } from "@/shell/primitives";

export interface SkillsLibraryProps {
  skills: SkillInstallation[];
  state: ActivityState;
  error?: string | null;
  /** Turns a skill on or off for one adapter, in this repository. */
  onToggleAdapter: (skill: SkillInstallation, adapter: "claude" | "codex", on: boolean) => void;
  onRemove: (skill: SkillInstallation) => void;
  busyId?: string | null;
}

const SCOPE_NOTE: Record<SkillScope, string> = {
  global: "available to every repository on this machine",
  project: "available to every task in this repository",
  task: "installed for one task only",
  workflow: "part of a workflow rather than picked by hand",
};

/**
 * What the agent is told, in plain sight.
 *
 * A skill is somebody else's instructions handed to an agent that acts on this
 * machine, so the whole text is here rather than a description of it — there is
 * no summary of an instruction that is safe to trust in place of the
 * instruction. The pinned hash is shown next to it: what runs is the content
 * that was inspected, not whatever the source has published since.
 */
export function SkillsLibrary({
  busyId,
  error,
  onRemove,
  onToggleAdapter,
  skills,
  state,
}: SkillsLibraryProps) {
  const [expanded, setExpanded] = useState<string | null>(skills[0]?.id ?? null);

  if (state === "loading") return <StateBlock state="loading" title="Reading installed skills" />;
  if (state === "failed") {
    return (
      <StateBlock
        state="failed"
        title="Skills could not be read"
        description={error ?? "Nothing was changed."}
      />
    );
  }
  if (skills.length === 0) {
    return (
      <StateBlock
        state="idle"
        title="No skills installed"
        description="A skill adds instructions to what an agent is told. Nothing is installed by default, and nothing installs itself."
      />
    );
  }

  return (
    <ul aria-label="Skills" className="flex flex-col">
      {skills.map((skill) => {
        const open = expanded === skill.id;
        return (
          <li key={skill.id} className="border-b border-border">
            <button
              type="button"
              aria-expanded={open}
              onClick={() => setExpanded(open ? null : skill.id)}
              className="flex w-full flex-col gap-0.5 px-3 py-2 text-left outline-none hover:bg-surface-raised focus-visible:bg-surface-raised"
            >
              <div className="flex flex-wrap items-baseline gap-2">
                <span className="text-[12px] text-foreground">{skill.name}</span>
                <Mono className="text-faint">{skill.scope}</Mono>
                <Mono className="ml-auto text-faint">{skill.pinnedHash.slice(0, 12)}</Mono>
              </div>
              <span className="text-[11px] text-muted-foreground">{SCOPE_NOTE[skill.scope]}</span>
            </button>

            {open ? (
              <div className="flex flex-col gap-2 border-t border-border bg-sunken px-3 py-2">
                <div className="flex flex-col gap-1">
                  <MetaLabel>What the agent is told</MetaLabel>
                  <pre className="max-h-72 overflow-auto whitespace-pre-wrap font-mono text-[11px] leading-relaxed text-muted-foreground">
                    {skill.skillMarkdown}
                  </pre>
                </div>

                <div className="flex flex-col gap-1">
                  <MetaLabel>When it applies</MetaLabel>
                  <p className="text-[11px] text-muted-foreground">
                    Picked by hand from the composer with <Mono className="text-faint">/</Mono>, or
                    already in scope for a task it was installed for. Mars does not trigger a skill
                    on its own.
                  </p>
                </div>

                <div className="flex flex-col gap-1">
                  <MetaLabel>Which agents may use it</MetaLabel>
                  <div className="flex flex-wrap items-center gap-1">
                    {(["claude", "codex"] as const).map((adapter) => {
                      const on = adapter === "claude" ? skill.allowClaude : skill.allowCodex;
                      return (
                        <button
                          key={adapter}
                          type="button"
                          aria-pressed={on}
                          disabled={busyId === skill.id}
                          onClick={() => onToggleAdapter(skill, adapter, !on)}
                          className={cn(
                            "flex items-center gap-1 border px-1.5 py-0.5 text-[11px] outline-none",
                            on
                              ? "border-primary text-foreground"
                              : "border-border text-muted-foreground hover:border-border-strong",
                            busyId === skill.id && "cursor-not-allowed text-faint",
                          )}
                        >
                          {on ? (
                            <PowerIcon className="size-3" />
                          ) : (
                            <PowerOffIcon className="size-3" />
                          )}
                          {adapter}
                        </button>
                      );
                    })}
                    <button
                      type="button"
                      disabled={busyId === skill.id}
                      onClick={() => onRemove(skill)}
                      className="ml-auto border border-border px-1.5 py-0.5 text-[11px] text-destructive outline-none hover:border-destructive disabled:cursor-not-allowed disabled:text-faint"
                    >
                      Uninstall
                    </button>
                  </div>
                </div>

                <Mono className="break-all text-faint">{skill.installPath}</Mono>
              </div>
            ) : null}
          </li>
        );
      })}
    </ul>
  );
}
