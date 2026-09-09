/**
 * Which rows of a long list are worth rendering.
 *
 * The transcript is the one surface with no natural ceiling: a 2,000-event
 * conversation is an ordinary day, every row can expand to its evidence, and
 * some of them mount a syntax highlighter when they do. Rendering all of it
 * costs a second of layout on every append.
 *
 * This is deliberately not a general virtualizer. Log rows are of wildly
 * different heights — a one-line command beside a forty-line failure — so
 * nothing here assumes a fixed row height. It measures what it has rendered,
 * remembers it, and estimates only what it has never seen. A row measured once
 * keeps its real height forever, so scrolling back over the same ground does not
 * make the scrollbar jump.
 */

export interface WindowRequest {
  /** How many rows the list holds in total. */
  count: number;
  scrollTop: number;
  viewportHeight: number;
  /** Height for a row that has never been on screen. */
  estimatedRowHeight: number;
  /**
   * Extra rows kept above and below the viewport.
   *
   * Not an optimisation: it is what stops a fast scroll showing blank space,
   * and what lets the browser's own find-in-page reach a row just off screen.
   */
  overscan?: number;
  /** Real heights for rows that have been measured, by index. */
  measured?: ReadonlyMap<number, number>;
}

export interface WindowResult {
  /** First index to render, inclusive. */
  start: number;
  /** Last index to render, exclusive. */
  end: number;
  /** Height of everything before `start`, as a spacer. */
  paddingTop: number;
  /** Height of everything after `end`, as a spacer. */
  paddingBottom: number;
  /** The whole list's height, measured where known and estimated elsewhere. */
  totalHeight: number;
}

export const DEFAULT_OVERSCAN = 8;

function heightAt(
  index: number,
  measured: ReadonlyMap<number, number> | undefined,
  estimated: number,
): number {
  return measured?.get(index) ?? estimated;
}

/**
 * The rows to render, and the space to leave for the ones that are not.
 *
 * Linear in the number of rows rather than binary-search over a prefix-sum
 * tree, because the caller only ever holds one page of the record at a time —
 * a few hundred rows — and a loop over that is far cheaper than keeping a
 * balanced structure correct as heights are measured.
 */
export function windowFor({
  count,
  estimatedRowHeight,
  measured,
  overscan = DEFAULT_OVERSCAN,
  scrollTop,
  viewportHeight,
}: WindowRequest): WindowResult {
  if (count <= 0) {
    return { start: 0, end: 0, paddingTop: 0, paddingBottom: 0, totalHeight: 0 };
  }

  const top = Math.max(0, scrollTop);
  const bottom = top + Math.max(0, viewportHeight);

  let offset = 0;
  let start = 0;
  let end = count;
  let foundStart = false;

  for (let index = 0; index < count; index += 1) {
    const height = heightAt(index, measured, estimatedRowHeight);
    if (!foundStart && offset + height > top) {
      start = index;
      foundStart = true;
    }
    if (foundStart && offset > bottom) {
      end = index;
      break;
    }
    offset += height;
  }
  if (!foundStart) {
    // Scrolled past everything — which happens for one frame after rows are
    // removed. Show the last page rather than nothing.
    start = Math.max(0, count - 1);
    end = count;
  }

  start = Math.max(0, start - overscan);
  end = Math.min(count, end + overscan);

  let paddingTop = 0;
  for (let index = 0; index < start; index += 1) {
    paddingTop += heightAt(index, measured, estimatedRowHeight);
  }
  let paddingBottom = 0;
  for (let index = end; index < count; index += 1) {
    paddingBottom += heightAt(index, measured, estimatedRowHeight);
  }
  let rendered = 0;
  for (let index = start; index < end; index += 1) {
    rendered += heightAt(index, measured, estimatedRowHeight);
  }

  return {
    start,
    end,
    paddingTop,
    paddingBottom,
    totalHeight: paddingTop + rendered + paddingBottom,
  };
}

/**
 * Whether a new window is different enough to be worth a re-render.
 *
 * Scroll fires far more often than the window actually changes, and re-rendering
 * a few hundred rows to move a spacer by two pixels is exactly the cost this
 * module exists to avoid.
 */
export function windowChanged(previous: WindowResult, next: WindowResult): boolean {
  return (
    previous.start !== next.start ||
    previous.end !== next.end ||
    Math.abs(previous.paddingTop - next.paddingTop) > 1 ||
    Math.abs(previous.paddingBottom - next.paddingBottom) > 1
  );
}
