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
import { useShellKeymap } from "@/shell/keymap";
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
  conversation = {},
  inspector = {},
  status = {},
  taskRail = { tasks: [] },
  utility = {},
  workbench = {},
}: ShellFrameProps) {
  useShellKeymap();
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
                    title={
                      workbenchKey === "editor" ? "No file open" : "No changes to review"
                    }
                    description={
                      workbenchKey === "editor"
                        ? "Open a file from a task, a diff, or a brain answer. The conversation stays where it is."
                        : "Staged and unstaged changes in the current worktree appear here."
                    }
                  />
                )}
              </div>
            </Region>
          </ResizablePanel>
        ) : null}
        {workbenchKey ? <ResizableHandle key="workbench-handle" withHandle /> : null}

        <ResizablePanel id="conversation" minSize="26%">
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
        <AppRail accountLabel={accountLabel} />
        <TaskRail {...taskRail} />

        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          {layout.utilityOpen ? (
            <ResizablePanelGroup className="min-h-0 flex-1" orientation="vertical">
              <ResizablePanel id="main" minSize="30%">
                {mainRow}
              </ResizablePanel>
              <ResizableHandle withHandle />
              <ResizablePanel
                id="utility"
                defaultSize="30%"
                minSize="12%"
                maxSize="70%"
                groupResizeBehavior="preserve-pixel-size"
              >
                <UtilityBody {...utility} />
              </ResizablePanel>
            </ResizablePanelGroup>
          ) : (
            <div className="flex min-h-0 flex-1">{mainRow}</div>
          )}

          <UtilityTabStrip statuses={utility.statuses} />
        </div>
      </div>

      <StatusBar {...status} />
    </div>
  );
}
