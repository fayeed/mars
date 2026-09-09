import { useCallback, useEffect, useRef } from "react";

import { useShell } from "@/shell/shell-store";

/**
 * Everything a keyboard can land on, in document order.
 *
 * Deliberately a query rather than a walk of every node: the list is only ever
 * needed for the two ends of a dialog, and anything with a negative tabindex is
 * reachable by script but not by Tab, which is exactly the distinction that
 * matters here.
 */
const FOCUSABLE = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

export function focusableWithin(container: HTMLElement): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    // Declared invisibility only. Measuring layout would be the more thorough
    // test and the wrong one here: `offsetParent` is null for a fixed-position
    // element, which is exactly what a dialog is, so a layout check would
    // quietly report that a dialog has nothing focusable in it at all.
    (element) => !element.closest("[hidden],[aria-hidden='true']"),
  );
}

/**
 * Where Tab should go from here, given a dialog's boundary.
 *
 * Returns `null` when the browser's own answer is already right — the point is
 * to wrap at the two ends, not to reimplement tabbing in between.
 */
export function wrapTabTarget(
  order: readonly HTMLElement[],
  active: Element | null,
  backwards: boolean,
): HTMLElement | null {
  if (order.length === 0) return null;
  const first = order[0];
  const last = order[order.length - 1];
  const index = order.indexOf(active as HTMLElement);
  // Focus somewhere outside the dialog: pull it back to the near end rather
  // than letting the next Tab escape into the shell behind.
  if (index === -1) return backwards ? last : first;
  if (!backwards && active === last) return first;
  if (backwards && active === first) return last;
  return null;
}

export interface ModalFocusOptions {
  /** Escape, the backdrop, and the close control all route here. */
  onClose: () => void;
  /** Skip every effect while the surface is not on screen. */
  active?: boolean;
}

/**
 * Focus for a modal surface: contained while it is open, given back when it
 * closes.
 *
 * The two halves matter equally. Containment is not a trap — Escape always
 * leaves, and the shell's own chords stand down while this is mounted so they
 * cannot fling focus to a region hidden behind the dialog. Restoration is what
 * makes the dialog a detour rather than a dead end: the reader ends up on the
 * control they opened it from, not on `document.body` with the next Tab
 * starting again from the top of the page.
 */
export function useModalFocus<T extends HTMLElement>({
  active = true,
  onClose,
}: ModalFocusOptions) {
  const ref = useRef<T | null>(null);
  const { pushModal } = useShell();
  // Held in a ref so the close handler stays stable across renders; a dialog
  // that re-subscribed its key listener on every keystroke would be fine, but
  // the restore target must not be recomputed after focus has already moved.
  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  useEffect(() => {
    if (!active) return;
    // Captured now, and used again at cleanup: by then React has cleared the
    // ref, and the question being asked — "is focus still inside what is
    // closing?" — is about this node either way.
    const container = ref.current;
    const restoreTo = document.activeElement as HTMLElement | null;
    const release = pushModal();

    if (container) {
      const order = focusableWithin(container);
      (order[0] ?? container).focus();
    }

    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onCloseRef.current();
        return;
      }
      if (event.key !== "Tab" || !ref.current) return;
      const target = wrapTabTarget(
        focusableWithin(ref.current),
        document.activeElement,
        event.shiftKey,
      );
      if (!target) return;
      event.preventDefault();
      target.focus();
    }

    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("keydown", onKeyDown, true);
      release();
      // Only take focus back if it is still ours to give. A dialog that closed
      // because the reader clicked something else must not yank them back.
      if (!restoreTo || !restoreTo.isConnected) return;
      if (container?.isConnected && !container.contains(document.activeElement)) return;
      restoreTo.focus();
    };
  }, [active, pushModal]);

  /** Closes when the backdrop itself is pressed, never a child of it. */
  const onBackdropMouseDown = useCallback((event: { target: unknown; currentTarget: unknown }) => {
    if (event.target === event.currentTarget) onCloseRef.current();
  }, []);

  return { ref, onBackdropMouseDown };
}
