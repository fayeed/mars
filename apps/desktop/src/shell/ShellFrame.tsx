import type { ReactNode } from "react";

import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable";
import type { ActivityState } from "@/shell/activity-state";
import { AppRail } from "@/shell/AppRail";
import { ConversationColumn, type ConversationColumnProps } from "@/shell/ConversationColumn";
import { Inspector, type InspectorProps } from "@/shell/Inspector";
import { useShellKeymap, type ShellKeymapHandlers } from "@/shell/keymap";
import { Mono, PanelHeader, StateBlock } from "@/shell/primitives";
import { Region } from "@/shell/Region";
import { useShell, type Destination } from "@/shell/shell-store";
import { StatusBar, type StatusBarProps } from "@/shell/StatusBar";
import { TaskRail, type TaskRailProps } from "@/shell/TaskRail";
import { UtilityBody, UtilityTabStrip, type UtilityPanelProps } from "@/shell/UtilityPanel";

export interface WorkbenchSurface {
  content?: ReactNode;
  state?: ActivityState;
  /** Machine-authored header detail: the open path, the compared refs. */
  detail?: string;
}

export type WorkbenchSurfaces = Partial<
  Record<Exclude<Destination, "conversation">, WorkbenchSurface>
>;

export interface ShellFrameProps {
  accountLabel?: string;
  /** Counts of what is waiting on the person, per destination. */
  badges?: Partial<Record<Destination, number>>;
  onAccount?: () => void;
  /**
   * The shell's own chords that need a screen to act on.
   *
   * They live in `SHELL_BINDINGS` with everything else rather than in a
   * hand-rolled listener, so they appear in the shortcut sheet, honour the
   * platform's modifier, and stand down behind a dialog like the rest.
   */
  keys?: ShellKeymapHandlers;
  conversation?: ConversationColumnProps;
  inspector?: InspectorProps;
  status?: StatusBarProps;
  taskRail?: TaskRailProps;
  utility?: UtilityPanelProps;
  workbench?: WorkbenchSurfaces;
}

const WORKBENCH_TITLE: Record<Exclude<Destination, "conversation">, string> = {
  editor: "Editor",
  "source-control": "Source control",
  agents: "Agents",
  knowledge: "Knowledge",
};

const WORKBENCH_EMPTY: Record<
  Exclude<Destination, "conversation">,
  { title: string; description: string }
> = {
  editor: {
    title: "No file open",
    description:
      "Open a file from a task, a diff, or a brain answer. The conversation stays where it is.",
  },
  "source-control": {
    title: "No changes to review",
    description: "Staged and unstaged changes in the current worktree appear here.",
  },
  agents: {
    title: "No agents yet",
    description: "Every agent run in this repository appears here, working or finished.",
  },
  knowledge: {
    title: "Nothing recorded yet",
    description:
      "Decisions agents make, memories they propose, and the project's code graph appear here.",
  },
};

/**
 * The Mars shell. Every screen in the product composes these regions; none of
 * them supplies its own chrome.
 *
 * The three destinations do not replace the conversation — `editor` and
 * `source-control` open a workbench beside it. That is what keeps the composer
 * usable while a diff is on screen.
 */
export function ShellFrame({
  accountLabel,
  badges,
  onAccount,
  keys,
  conversation = {},
  inspector = {},
  status = {},
  taskRail = { tasks: [] },
  utility = {},
  workbench = {},
}: ShellFrameProps) {
  useShellKeymap(keys);
  const { layout } = useShell();

  const workbenchKey =
    layout.destination === "conversation"
      ? null
      : (layout.destination as Exclude<Destination, "conversation">);
  const surface = workbenchKey ? workbench[workbenchKey] : undefined;
  const inspectorExpanded = !layout.inspectorCollapsed;

  const mainRow = (
    <div className="flex min-h-0 min-w-0 flex-1">
      <ResizablePanelGroup className="min-h-0 min-w-0 flex-1" orientation="horizontal">
        {workbenchKey ? (
          <ResizablePanel
            key="workbench"
            id="workbench"
            defaultSize="46%"
            minSize="22%"
            groupResizeBehavior="preserve-pixel-size"
          >
            <Region
              label={WORKBENCH_TITLE[workbenchKey]}
              region="workbench"
              className="h-full border-r border-border bg-background"
            >
              <PanelHeader
                title={WORKBENCH_TITLE[workbenchKey]}
                state={surface?.state}
                actions={surface?.detail ? <Mono className="text-faint">{surface.detail}</Mono> : undefined}
              />
              <div className="min-h-0 flex-1 overflow-auto">
                {surface?.content ?? (
                  <StateBlock
                    state={surface?.state ?? "idle"}
                    title={WORKBENCH_EMPTY[workbenchKey].title}
                    description={WORKBENCH_EMPTY[workbenchKey].description}
                  />
                )}
              </div>
            </Region>
          </ResizablePanel>
        ) : null}
        {workbenchKey ? <ResizableHandle key="workbench-handle" withHandle /> : null}

        <ResizablePanel key="conversation" id="conversation" minSize="26%">
          <ConversationColumn {...conversation} />
        </ResizablePanel>

        {inspectorExpanded ? <ResizableHandle key="inspector-handle" withHandle /> : null}
        {inspectorExpanded ? (
          <ResizablePanel
            key="inspector"
            id="inspector"
            defaultSize="24%"
            minSize="16%"
            maxSize="42%"
            groupResizeBehavior="preserve-pixel-size"
          >
            <Inspector {...inspector} />
          </ResizablePanel>
        ) : null}
      </ResizablePanelGroup>

      {inspectorExpanded ? null : <Inspector {...inspector} />}
    </div>
  );

  return (
    <div className="flex h-screen min-w-[820px] flex-col overflow-hidden bg-sunken text-foreground">
      <div className="flex min-h-0 flex-1">
        <AppRail accountLabel={accountLabel} badges={badges} onAccount={onAccount} />
        <TaskRail {...taskRail} />

        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          {/*
            One tree, always. Swapping between a panel group and a plain div
            when the utility panel opened re-parented everything below it, so
            React unmounted and rebuilt the whole main row — Monaco, xterm, every
            expanded log row, and the conversation's scroll position with them.
            The panel is added and removed as a sibling *after* the main row
            instead, which leaves the main row exactly where it was.
          */}
          <ResizablePanelGroup className="min-h-0 flex-1" orientation="vertical">
            <ResizablePanel key="main" id="main" minSize="30%">
              {mainRow}
            </ResizablePanel>
            {layout.utilityOpen ? <ResizableHandle key="utility-handle" withHandle /> : null}
            {layout.utilityOpen ? (
              <ResizablePanel
                key="utility"
                id="utility"
                defaultSize="30%"
                minSize="12%"
                maxSize="70%"
                groupResizeBehavior="preserve-pixel-size"
              >
                <UtilityBody {...utility} />
              </ResizablePanel>
            ) : null}
          </ResizablePanelGroup>

          <UtilityTabStrip statuses={utility.statuses} />
        </div>
      </div>

      <StatusBar {...status} />
    </div>
  );
}
