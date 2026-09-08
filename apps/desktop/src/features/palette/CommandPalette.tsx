import type { PaletteHit } from "@mars/shared-types";
import { useEffect, useMemo, useRef, useState } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { KeyHint, MetaLabel, Mono, StateLabel } from "@/shell/primitives";

export interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
  /** Debounced by the caller; this component asks on every keystroke. */
  onSearch: (query: string) => void;
  hits: PaletteHit[];
  state: ActivityState;
  onJump: (hit: PaletteHit) => void;
}

/** The order groups are shown in: nearest to the work first. */
const GROUP_ORDER: PaletteHit["kind"][] = ["task", "message", "file", "symbol", "memory"];

const GROUP_LABEL: Record<PaletteHit["kind"], string> = {
  task: "Tasks",
  message: "Messages",
  file: "Files",
  symbol: "Symbols",
  memory: "Memories",
};

/**
 * One way to reach anything.
 *
 * Results are grouped rather than interleaved, because ranking a file name
 * against an approved memory would be a made-up comparison and the reader can
 * see the difference at a glance anyway. Everything is reachable from the
 * keyboard alone: the arrow keys walk the flattened list across group
 * boundaries, so no group can trap the cursor.
 */
export function CommandPalette({
  hits,
  onClose,
  onJump,
  onSearch,
  open,
  state,
}: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [highlight, setHighlight] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);

  const grouped = useMemo(() => {
    const buckets = new Map<PaletteHit["kind"], PaletteHit[]>();
    for (const hit of hits) {
      const list = buckets.get(hit.kind) ?? [];
      list.push(hit);
      buckets.set(hit.kind, list);
    }
    return GROUP_ORDER.filter((kind) => buckets.has(kind)).map(
      (kind) => [kind, buckets.get(kind) ?? []] as const,
    );
  }, [hits]);

  // The flattened order the arrow keys walk. Grouping is presentation; moving
  // through results should not depend on how they were bucketed.
  const flat = useMemo(() => grouped.flatMap(([, items]) => items), [grouped]);

  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  if (!open) return null;

  const active = flat[Math.min(highlight, Math.max(0, flat.length - 1))];

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-sunken/70 pt-24"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-label="Search everything"
        aria-modal="true"
        className="flex max-h-[60vh] w-[min(44rem,92vw)] flex-col border border-border bg-surface"
      >
        <div className="flex items-center gap-2 border-b border-border px-2 py-1.5">
          <input
            ref={inputRef}
            aria-label="Search everything"
            aria-controls="palette-results"
            aria-expanded
            role="combobox"
            autoComplete="off"
            className="min-w-0 flex-1 bg-transparent text-[13px] text-foreground outline-none placeholder:text-faint"
            onChange={(event) => {
              setQuery(event.target.value);
              setHighlight(0);
              onSearch(event.target.value);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                onClose();
                return;
              }
              if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                event.preventDefault();
                if (flat.length === 0) return;
                const step = event.key === "ArrowDown" ? 1 : -1;
                setHighlight((current) => (current + step + flat.length) % flat.length);
                return;
              }
              if (event.key === "Enter" && active) {
                event.preventDefault();
                onJump(active);
              }
            }}
            placeholder="Tasks, messages, files, symbols, memories"
            value={query}
          />
          <KeyHint keys="Esc" />
        </div>

        <div id="palette-results" role="listbox" aria-label="Results" className="min-h-0 flex-1 overflow-y-auto">
          {state === "loading" ? (
            <div className="px-3 py-3">
              <StateLabel state="loading" detail="searching" />
            </div>
          ) : query.trim().length === 0 ? (
            <p className="px-3 py-3 text-[11px] text-muted-foreground">
              Type to search. Results are grouped by what they are, not ranked against each other.
            </p>
          ) : flat.length === 0 ? (
            <p className="px-3 py-3 text-[11px] text-muted-foreground">
              Nothing matched. Files and symbols come from the index, so anything unindexed will not
              appear here.
            </p>
          ) : (
            grouped.map(([kind, items]) => (
              <section key={kind}>
                <div className="flex items-baseline gap-2 border-b border-border px-3 pt-2 pb-1">
                  <MetaLabel>{GROUP_LABEL[kind]}</MetaLabel>
                  <Mono className="text-faint">{items.length}</Mono>
                </div>
                <ul>
                  {items.map((hit) => {
                    const index = flat.indexOf(hit);
                    const selected = index === highlight;
                    return (
                      <li
                        key={`${hit.kind}:${hit.target}:${hit.sequence ?? hit.line ?? hit.label}`}
                        role="option"
                        aria-selected={selected}
                        onMouseDown={(event) => {
                          event.preventDefault();
                          onJump(hit);
                        }}
                        onMouseEnter={() => setHighlight(index)}
                        className={cn(
                          "flex cursor-pointer items-baseline gap-2 px-3 py-1",
                          selected && "bg-surface-raised",
                        )}
                      >
                        <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">
                          {hit.label}
                        </span>
                        <Mono className="max-w-64 shrink-0 truncate text-faint">{hit.detail}</Mono>
                      </li>
                    );
                  })}
                </ul>
              </section>
            ))
          )}
        </div>

        <div className="flex items-center gap-2 border-t border-border px-2 py-1">
          <KeyHint keys="↑↓" />
          <span className="text-[11px] text-faint">move</span>
          <KeyHint keys="Enter" />
          <span className="text-[11px] text-faint">jump</span>
        </div>
      </div>
    </div>
  );
}
