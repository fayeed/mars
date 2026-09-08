import type { ReactNode } from "react";

import type { ActivityState } from "@/shell/activity-state";
import { Mono, PanelHeader, StateBlock, StateLabel } from "@/shell/primitives";
import { Region } from "@/shell/Region";
import { useScrollAnchor } from "@/shell/scroll-anchor";
import { useShell } from "@/shell/shell-store";

export interface ConversationColumnProps {
  title?: ReactNode;
  /** Machine-authored subtitle: branch, worktree, elapsed time. */
  detail?: string;
  state?: ActivityState;
  children?: ReactNode;
  composer?: ReactNode;
  emptyTitle?: string;
  emptyDescription?: string;
}

/**
 * The conversation is the one region that is always present and always live.
 * Destinations open beside it; panels open around it; it is never replaced and
 * never scrolled out from under the reader when the layout changes.
 */
export function ConversationColumn({
  children,
  composer,
  detail,
  emptyDescription = "Everything an agent does lands here, in order, with the reason it happened.",
  emptyTitle = "No conversation yet",
  state = "idle",
  title = "Conversation",
}: ConversationColumnProps) {
  const { layout } = useShell();

  // Any region opening, closing or switching changes the conversation's height
  // or width. The anchor compensates so the reader keeps their place.
  const layoutKey = [
    layout.destination,
    layout.taskRailCollapsed,
    layout.inspectorCollapsed,
    layout.utilityOpen,
    layout.utilityTab,
  ].join("|");
  const { capture, ref } = useScrollAnchor<HTMLDivElement>(layoutKey);

  return (
    <Region label="Conversation" region="conversation" className="h-full bg-background">
      <PanelHeader
        title={title}
        actions={detail ? <Mono className="text-faint">{detail}</Mono> : undefined}
      >
        {state === "idle" ? null : (
          <StateLabel className="justify-end pr-2" state={state} />
        )}
      </PanelHeader>

      <div
        ref={ref}
        onScroll={capture}
        data-slot="conversation-scroll"
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain"
      >
        {children ?? (
          <StateBlock state={state} title={emptyTitle} description={emptyDescription} />
        )}
      </div>

      {composer}
    </Region>
  );
}
