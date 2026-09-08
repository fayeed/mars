import type { ReactNode } from "react";

import { cn } from "@/lib/utils";
import { KeyHint, MetaLabel } from "@/shell/primitives";

export type LibraryTab = "memories" | "skills" | "settings";

export interface LibraryOverlayProps {
  open: boolean;
  tab: LibraryTab;
  onTab: (tab: LibraryTab) => void;
  onClose: () => void;
  memories: ReactNode;
  skills: ReactNode;
  settings: ReactNode;
}

const TABS: { id: LibraryTab; label: string; note: string }[] = [
  { id: "memories", label: "Memories", note: "what Mars has been told to remember" },
  { id: "skills", label: "Skills", note: "what agents are told" },
  { id: "settings", label: "Settings", note: "agents, credentials, permissions, data" },
];

/**
 * The management surfaces, which are not destinations.
 *
 * The app rail names the three places the product can be, and memories, skills
 * and settings are none of them — they are things you go and look at, then
 * leave. Putting them behind one overlay keeps the rail honest about where you
 * are while still making all three reachable from anywhere, and closing it puts
 * you back exactly where you were.
 */
export function LibraryOverlay({
  memories,
  onClose,
  onTab,
  open,
  settings,
  skills,
  tab,
}: LibraryOverlayProps) {
  if (!open) return null;

  const active = TABS.find((candidate) => candidate.id === tab) ?? TABS[0];

  return (
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-sunken/70 p-8"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Library"
        className="flex h-full w-[min(60rem,96vw)] flex-col border border-border bg-surface"
      >
        <header className="flex shrink-0 items-center gap-1 border-b border-border px-2 py-1.5">
          <div role="tablist" aria-label="Library sections" className="flex items-center gap-1">
            {TABS.map((candidate) => (
              <button
                key={candidate.id}
                type="button"
                role="tab"
                aria-selected={candidate.id === tab}
                onClick={() => onTab(candidate.id)}
                className={cn(
                  "border px-2 py-0.5 text-[11px] outline-none",
                  candidate.id === tab
                    ? "border-primary text-foreground"
                    : "border-border text-muted-foreground hover:border-border-strong",
                )}
              >
                {candidate.label}
              </button>
            ))}
          </div>
          <MetaLabel className="ml-2 truncate">{active.note}</MetaLabel>
          <button
            type="button"
            onClick={onClose}
            className="ml-auto flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong"
          >
            Close
            <KeyHint className="border-transparent" keys="Esc" />
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-hidden bg-background">
          {tab === "memories" ? memories : tab === "skills" ? skills : settings}
        </div>
      </div>
    </div>
  );
}
