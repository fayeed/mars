import { useCallback, useLayoutEffect, useRef } from "react";

export interface ScrollMetrics {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}

/** Within this many pixels of the bottom counts as "following the stream". */
export const PINNED_THRESHOLD = 24;

export function isPinnedToBottom(metrics: ScrollMetrics, threshold = PINNED_THRESHOLD): boolean {
  return metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight <= threshold;
}

/**
 * Where the conversation should sit after the viewport changed size.
 *
 * Opening the utility panel shortens the conversation. Left alone, the browser
 * keeps `scrollTop` and the reader's line slides down the screen — the panel
 * appears to have stolen their place. Anchoring on distance-from-bottom keeps
 * the line they were reading where it was, and a reader who was following the
 * stream stays pinned to the newest message.
 */
export function anchoredScrollTop(
  previous: ScrollMetrics,
  next: Omit<ScrollMetrics, "scrollTop">,
  threshold = PINNED_THRESHOLD,
): number {
  const maximum = Math.max(0, next.scrollHeight - next.clientHeight);
  if (isPinnedToBottom(previous, threshold)) return maximum;
  const distanceFromBottom = previous.scrollHeight - previous.scrollTop - previous.clientHeight;
  return Math.min(maximum, Math.max(0, maximum - distanceFromBottom));
}

function readMetrics(element: HTMLElement): ScrollMetrics {
  return {
    scrollTop: element.scrollTop,
    scrollHeight: element.scrollHeight,
    clientHeight: element.clientHeight,
  };
}

/**
 * Preserves the conversation's reading position across shell layout changes.
 * `layoutKey` should change whenever a region opens, closes or resizes.
 */
export function useScrollAnchor<T extends HTMLElement>(layoutKey: unknown) {
  const ref = useRef<T | null>(null);
  const metricsRef = useRef<ScrollMetrics | null>(null);
  const firstRunRef = useRef(true);

  const capture = useCallback(() => {
    if (ref.current) metricsRef.current = readMetrics(ref.current);
  }, []);

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    if (firstRunRef.current) {
      firstRunRef.current = false;
      metricsRef.current = readMetrics(element);
      return;
    }
    const previous = metricsRef.current;
    if (!previous) return;
    element.scrollTop = anchoredScrollTop(previous, {
      scrollHeight: element.scrollHeight,
      clientHeight: element.clientHeight,
    });
    metricsRef.current = readMetrics(element);
  }, [layoutKey]);

  return { ref, capture };
}
