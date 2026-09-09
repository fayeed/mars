import { useState, type ReactNode } from "react";

import type { ActivityState } from "@/shell/activity-state";
import { Mono, PanelHeader, StateBlock, StateLabel } from "@/shell/primitives";
import { Region } from "@/shell/Region";
import { useScrollAnchor } from "@/shell/scroll-anchor";
import { useShell } from "@/shell/shell-store";

export interface ConversationColumnProps {
  title?: ReactNode;
  /**
   * The children, given the element they scroll inside.
   *
   * A long transcript has to know its own scroller to render only the rows near
   * the viewport, and the scroller is this column — the same element the anchor
   * compensates on when a panel opens. Giving the transcript one of its own
   * would put two scrollbars on screen and split the reading position between
   * them.
   */
  renderChildren?: (scroller: HTMLElement | null) => ReactNode;
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
  renderChildren,
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
  // The ref is set during commit, which is too late for the first render of the
  // children that need it; this state re-renders them once it exists.
  const [scroller, setScroller] = useState<HTMLDivElement | null>(null);

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
        ref={(element) => {
          ref.current = element;
          setScroller((current) => (current === element ? current : element));
        }}
        onScroll={capture}
        data-slot="conversation-scroll"
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain"
      >
        {renderChildren?.(scroller) ??
          children ?? <StateBlock state={state} title={emptyTitle} description={emptyDescription} />}
      </div>

      {composer}
    </Region>
  );
}
