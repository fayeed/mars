import { useState, type ReactNode } from "react";

import { cn } from "@/lib/utils";
import { Mono } from "@/shell/primitives";

export type KnowledgeTab = "decisions" | "memory" | "code";

export interface KnowledgeViewProps {
  decisions: ReactNode;
  memory: ReactNode;
  /** The code graph; absent until a repository has been indexed. */
  code?: ReactNode;
  /** How many decisions and memory proposals are waiting on the person. */
  waiting: { decisions: number; memory: number };
  /** The tab shown first; the screen remounts this to open another. */
  initialTab?: KnowledgeTab;
}

const TABS: { id: KnowledgeTab; label: string }[] = [
  { id: "decisions", label: "Decisions" },
  { id: "memory", label: "Memory" },
  { id: "code", label: "Code graph" },
];

/**
 * What the project knows: the decisions agents made and you settled, what
 * Mars has been told to remember, and how the code fits together. The first
 * two are what agents are given as fact, so each tab says how much of it is
 * still waiting on you.
 */
export function KnowledgeView({
  code,
  decisions,
  initialTab,
  memory,
  waiting,
}: KnowledgeViewProps) {
  const [tab, setTab] = useState<KnowledgeTab>(
    initialTab ?? (waiting.decisions === 0 && waiting.memory > 0 ? "memory" : "decisions"),
  );
  const tabs = TABS.filter((candidate) => candidate.id !== "code" || code !== undefined);

  return (
    <div className="flex h-full min-h-0 flex-col" data-slot="knowledge-view">
      <div
        role="tablist"
        aria-label="Knowledge views"
        className="flex items-center gap-1 border-b border-border px-2 py-1"
      >
        {tabs.map(({ id, label }) => {
          const count = id === "decisions" ? waiting.decisions : id === "memory" ? waiting.memory : 0;
          return (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={tab === id}
              onClick={() => setTab(id)}
              className={cn(
                "flex items-baseline gap-1.5 border px-2 py-0.5 text-[11px] outline-none",
                tab === id
                  ? "border-primary text-foreground"
                  : "border-border text-muted-foreground hover:border-border-strong",
              )}
            >
              {label}
              {count > 0 ? (
                <Mono className="text-primary" aria-label={`${count} waiting`}>
                  {count}
                </Mono>
              ) : null}
            </button>
          );
        })}
      </div>
      <div role="tabpanel" className="min-h-0 flex-1 overflow-hidden">
        {tab === "decisions" ? decisions : tab === "memory" ? memory : code}
      </div>
    </div>
  );
}
