import { useCallback, useEffect, useRef, useState } from "react";

import {
  DEFAULT_OVERSCAN,
  windowChanged,
  windowFor,
  type WindowResult,
} from "@/shell/virtual-list";

const EMPTY: WindowResult = {
  start: 0,
  end: 0,
  paddingTop: 0,
  paddingBottom: 0,
  totalHeight: 0,
};

export interface VirtualRowsOptions {
  count: number;
  /** Height for a row that has never been on screen. */
  estimatedRowHeight?: number;
  overscan?: number;
  /**
   * The element that actually scrolls.
   *
   * Passed in rather than owned, because the transcript scrolls inside the
   * conversation column — the same element the scroll anchor compensates on —
   * and giving the list its own scroller would put two of them on screen.
   */
  scroller: HTMLElement | null;
  /** Switched off entirely below this many rows; see the note in the hook. */
  threshold?: number;
}

/**
 * How many rows are worth virtualizing for.
 *
 * Below this the machinery costs more than it saves, and it has a real price:
 * a virtualized row cannot be found by the browser's find-in-page, or reached
 * by a screen reader walking the document, until it is scrolled into view.
 * Paying that for forty rows would be a bad trade.
 */
export const VIRTUALIZE_ABOVE = 120;

/**
 * Render only the rows near the viewport, measuring them as they appear.
 *
 * Returns `null` when there is nothing to virtualize, which callers should read
 * as "render everything" — the plain path stays plain, and the whole list stays
 * searchable and reachable at the sizes where that matters most.
 */
export function useVirtualRows({
  count,
  estimatedRowHeight = 32,
  overscan = DEFAULT_OVERSCAN,
  scroller,
  threshold = VIRTUALIZE_ABOVE,
}: VirtualRowsOptions): {
  window: WindowResult | null;
  /** Attach to each rendered row so its real height is remembered. */
  measure: (index: number, element: HTMLElement | null) => void;
} {
  // Without a scroller there is nothing to measure against, and a window
  // computed from nothing would render nothing at all. Falling back to the whole
  // list is slower and correct; rendering an empty log is neither.
  const active = count > threshold && scroller !== null;
  const measured = useRef(new Map<number, number>());
  const [result, setResult] = useState<WindowResult>(EMPTY);
  const resultRef = useRef(result);
  useEffect(() => {
    resultRef.current = result;
  }, [result]);

  const recompute = useCallback(() => {
    if (!scroller || !active) return;
    const next = windowFor({
      count,
      scrollTop: scroller.scrollTop,
      viewportHeight: scroller.clientHeight,
      estimatedRowHeight,
      overscan,
      measured: measured.current,
    });
    // Scroll fires far more often than the window changes.
    if (windowChanged(resultRef.current, next)) setResult(next);
  }, [active, count, estimatedRowHeight, overscan, scroller]);

  useEffect(() => {
    if (!scroller || !active) return;
    // Computed from a listener rather than during render: reading `scrollTop`
    // and `clientHeight` is a layout read, and doing it in render would force a
    // synchronous reflow on every pass.
    const onScroll = () => recompute();
    scroller.addEventListener("scroll", onScroll, { passive: true });
    const observer = new ResizeObserver(onScroll);
    observer.observe(scroller);
    onScroll();
    return () => {
      scroller.removeEventListener("scroll", onScroll);
      observer.disconnect();
    };
  }, [active, recompute, scroller]);

  // A row that changes height — a log row expanding to its evidence — has to be
  // re-measured, or every spacer below it is wrong until the next scroll.
  const measure = useCallback(
    (index: number, element: HTMLElement | null) => {
      if (!element || !active) return;
      const height = element.getBoundingClientRect().height;
      if (height <= 0) return;
      const known = measured.current.get(index);
      if (known !== undefined && Math.abs(known - height) < 1) return;
      measured.current.set(index, height);
      recompute();
    },
    [active, recompute],
  );

  // Rows are addressed by index, so anything that reorders them invalidates
  // every measurement. The record only ever grows at the end, but a different
  // task's record replaces the list wholesale.
  useEffect(() => {
    measured.current.clear();
  }, [threshold]);

  return { window: active ? result : null, measure };
}
