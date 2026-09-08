import { PanelRightIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import { summarizeStates, type ActivityState } from "@/shell/activity-state";
import { bindingHint } from "@/shell/keymap";
import type { InspectorContent, InspectorStatuses } from "@/shell/model";
import { Mono, PanelHeader, RailButton, StateBlock, StateDot } from "@/shell/primitives";
import { Region } from "@/shell/Region";
import { INSPECTOR_TABS, useShell, type InspectorTab } from "@/shell/shell-store";

const TAB_LABEL: Record<InspectorTab, string> = {
  context: "Context",
  plan: "Plan",
  diff: "Diff",
  brain: "Brain",
};

export interface InspectorProps {
  content?: InspectorContent;
  statuses?: InspectorStatuses;
}

/**
 * The inspector answers "what is this run made of" — context pack, plan, diff
 * summary, brain answers. It only ever changes width when the user asks it to;
 * window resizing is absorbed by the conversation, never taken from it.
 */
export function Inspector({ content = {}, statuses = {} }: InspectorProps) {
  const { dispatch, layout } = useShell();
  const states = INSPECTOR_TABS.map((tab) => statuses[tab]?.state).filter(
    (state): state is ActivityState => Boolean(state),
  );

  if (layout.inspectorCollapsed) {
    return (
      <div className="flex w-[var(--shell-strip-width)] shrink-0 flex-col items-center gap-1 border-l border-border bg-surface py-1">
        <RailButton
          marker="none"
          size="strip"
          title={`Expand inspector · ${bindingHint("toggle.inspector")}`}
          aria-expanded={false}
          onClick={() => dispatch({ type: "openInspector" })}
        >
          <PanelRightIcon />
          <span className="sr-only">Expand inspector</span>
        </RailButton>
        {states.length > 0 ? <StateDot state={summarizeStates(states)} /> : null}
        <div className="flex flex-1 flex-col items-center gap-3 pt-2">
          {INSPECTOR_TABS.map((tab) => (
            <button
              key={tab}
              type="button"
              aria-current={layout.inspectorTab === tab ? "true" : undefined}
              onClick={() => dispatch({ type: "activateInspectorTab", tab })}
              className={cn(
                "shell-strip-label shell-label outline-none hover:text-foreground focus-visible:text-foreground focus-visible:shadow-[inset_0_0_0_1px_var(--ring)]",
                layout.inspectorTab === tab && "text-primary",
              )}
            >
              {TAB_LABEL[tab]}
            </button>
          ))}
        </div>
      </div>
    );
  }

  const status = statuses[layout.inspectorTab];
  const body = content[layout.inspectorTab];

  return (
    <Region
      as="aside"
      label="Inspector"
      region="inspector"
      className="h-full border-l border-border bg-surface"
    >
      <PanelHeader
        title="Inspector"
        actions={
          <RailButton
            marker="none"
            size="strip"
            aria-expanded
            title={`Collapse inspector · ${bindingHint("toggle.inspector")}`}
            onClick={() => dispatch({ type: "toggleInspector" })}
          >
            <PanelRightIcon />
            <span className="sr-only">Collapse inspector</span>
          </RailButton>
        }
      />

      <div role="tablist" aria-label="Inspector sections" className="flex border-b border-border">
        {INSPECTOR_TABS.map((tab) => {
          const tabStatus = statuses[tab];
          const selected = layout.inspectorTab === tab;
          return (
            <RailButton
              key={tab}
              role="tab"
              size="tab"
              marker="bottom"
              aria-selected={selected}
              aria-current={selected ? "page" : undefined}
              tabIndex={selected ? 0 : -1}
              onClick={() => dispatch({ type: "activateInspectorTab", tab })}
              onKeyDown={(event) => {
                if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
                event.preventDefault();
                const step = event.key === "ArrowRight" ? 1 : -1;
                const index = INSPECTOR_TABS.indexOf(tab);
                const next = INSPECTOR_TABS[(index + step + INSPECTOR_TABS.length) % INSPECTOR_TABS.length];
                dispatch({ type: "activateInspectorTab", tab: next });
              }}
            >
              {tabStatus ? <StateDot state={tabStatus.state} /> : null}
              {TAB_LABEL[tab]}
              {typeof tabStatus?.count === "number" ? (
                <Mono className="text-faint">{tabStatus.count}</Mono>
              ) : null}
            </RailButton>
          );
        })}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {body ?? (
          <StateBlock
            state={status?.state ?? "idle"}
            title={inspectorEmptyTitle(layout.inspectorTab, status?.state)}
            description={inspectorEmptyDescription(layout.inspectorTab, status?.state)}
          />
        )}
      </div>
    </Region>
  );
}

function inspectorEmptyTitle(tab: InspectorTab, state: ActivityState | undefined): string {
  if (state === "loading") return `Loading ${TAB_LABEL[tab].toLowerCase()}`;
  if (state === "failed") return `${TAB_LABEL[tab]} unavailable`;
  if (state === "stale") return `${TAB_LABEL[tab]} is out of date`;
  return `No ${TAB_LABEL[tab].toLowerCase()} yet`;
}

function inspectorEmptyDescription(
  tab: InspectorTab,
  state: ActivityState | undefined,
): string | undefined {
  if (state === "stale") return "The working tree moved since this was computed. Refresh to recompute.";
  if (state === "failed") return "Nothing was changed. The last good value is still on disk.";
  if (tab === "context") return "The context pack appears here before a run is launched, and stays inspectable after.";
  if (tab === "plan") return "A plan appears once an agent proposes one.";
  if (tab === "diff") return "Changes in the task worktree are summarized here.";
  return "Ask the project brain a question and the answer lands here with its sources.";
}
