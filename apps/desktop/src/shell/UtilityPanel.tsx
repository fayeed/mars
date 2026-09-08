import { ChevronDownIcon, ChevronUpIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { bindingHint } from "@/shell/keymap";
import type { UtilityContent, UtilityStatuses } from "@/shell/model";
import { Mono, RailButton, StateBlock, StateDot } from "@/shell/primitives";
import { Region } from "@/shell/Region";
import { UTILITY_TABS, useShell, type UtilityTab } from "@/shell/shell-store";

const TAB_LABEL: Record<UtilityTab, string> = {
  problems: "Problems",
  tests: "Tests",
  checks: "Checks",
  terminal: "Terminal",
};

export interface UtilityPanelProps {
  content?: UtilityContent;
  statuses?: UtilityStatuses;
}

/**
 * The tab strip never moves. Opening and closing the panel changes only the
 * height of the body above it, so the tabs stay where the hand left them and
 * the conversation's scroll anchor has one predictable thing to compensate for.
 */
export function UtilityTabStrip({ statuses = {} }: { statuses?: UtilityStatuses }) {
  const { dispatch, focusRegion, layout } = useShell();

  return (
    <div
      role="tablist"
      aria-label="Utility panel"
      className="flex h-[var(--shell-header-height)] shrink-0 items-stretch border-t border-border bg-surface"
    >
      {UTILITY_TABS.map((tab) => {
        const status = statuses[tab];
        const selected = layout.utilityOpen && layout.utilityTab === tab;
        return (
          <RailButton
            key={tab}
            role="tab"
            size="tab"
            marker="bottom"
            aria-selected={selected}
            aria-current={selected ? "page" : undefined}
            aria-controls="shell-utility-body"
            tabIndex={layout.utilityTab === tab ? 0 : -1}
            onClick={() => {
              dispatch({ type: "activateUtilityTab", tab });
              if (!(layout.utilityOpen && layout.utilityTab === tab)) focusRegion("utility");
            }}
            onKeyDown={(event) => {
              if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
              event.preventDefault();
              const step = event.key === "ArrowRight" ? 1 : -1;
              const index = UTILITY_TABS.indexOf(tab);
              dispatch({
                type: "openUtility",
                tab: UTILITY_TABS[(index + step + UTILITY_TABS.length) % UTILITY_TABS.length],
              });
            }}
          >
            {status ? <StateDot state={status.state} /> : null}
            {TAB_LABEL[tab]}
            {typeof status?.count === "number" ? (
              <Mono className="text-faint">{status.count}</Mono>
            ) : null}
          </RailButton>
        );
      })}

      <div className="flex-1" />

      <RailButton
        marker="none"
        size="strip"
        aria-expanded={layout.utilityOpen}
        title={`${layout.utilityOpen ? "Close" : "Open"} utility panel · ${bindingHint("toggle.utility")}`}
        onClick={() => dispatch({ type: "toggleUtility" })}
      >
        {layout.utilityOpen ? <ChevronDownIcon /> : <ChevronUpIcon />}
        <span className="sr-only">
          {layout.utilityOpen ? "Close utility panel" : "Open utility panel"}
        </span>
      </RailButton>
    </div>
  );
}

export function UtilityBody({ content = {}, statuses = {} }: UtilityPanelProps) {
  const { layout } = useShell();
  const tab = layout.utilityTab;
  const status = statuses[tab];
  const body = content[tab];

  return (
    <Region
      label={`${TAB_LABEL[tab]} panel`}
      region="utility"
      id="shell-utility-body"
      className={cn("h-full border-t border-border bg-background")}
    >
      <div className="min-h-0 flex-1 overflow-auto">
        {body ?? (
          <StateBlock
            state={status?.state ?? "idle"}
            title={utilityEmptyTitle(tab, status?.state)}
            description={utilityEmptyDescription(tab, status?.state)}
          />
        )}
      </div>
    </Region>
  );
}

function utilityEmptyTitle(tab: UtilityTab, state: ActivityState | undefined): string {
  if (state === "loading") return `Running ${TAB_LABEL[tab].toLowerCase()}`;
  if (state === "failed") return `${TAB_LABEL[tab]} failed`;
  if (state === "stale") return `${TAB_LABEL[tab]} are out of date`;
  if (tab === "terminal") return "No terminal running";
  return `No ${TAB_LABEL[tab].toLowerCase()}`;
}

function utilityEmptyDescription(
  tab: UtilityTab,
  state: ActivityState | undefined,
): string | undefined {
  if (state === "stale") return "The worktree changed after this ran. Run it again to trust the result.";
  if (tab === "terminal") return "A terminal starts only when you ask for one, in the trusted worktree root.";
  return undefined;
}
