import { useEffect, useRef } from "react";

import {
  DESTINATIONS,
  useShell,
  type ShellAction,
} from "@/shell/shell-store";

export interface KeyChord {
  key: string;
  /** Ctrl on Windows/Linux, Command on macOS. */
  mod?: boolean;
  shift?: boolean;
  alt?: boolean;
}

export interface ShellBinding {
  id: string;
  chord: KeyChord;
  label: string;
  hint: string;
}

const isApple =
  typeof navigator !== "undefined" && /mac|iphone|ipad/i.test(navigator.platform || navigator.userAgent);

export function chordHint(chord: KeyChord): string {
  const parts: string[] = [];
  if (chord.mod) parts.push(isApple ? "⌘" : "Ctrl");
  if (chord.alt) parts.push(isApple ? "⌥" : "Alt");
  if (chord.shift) parts.push("Shift");
  parts.push(chord.key.length === 1 ? chord.key.toUpperCase() : chord.key);
  return parts.join(isApple ? "" : "+");
}

function binding(id: string, chord: KeyChord, label: string): ShellBinding {
  return { id, chord, label, hint: chordHint(chord) };
}

/**
 * Every shell region can be reached and dismissed without a pointer. F6 walks
 * the regions in reading order the way a screen-reader user expects; the
 * mod-chords jump straight to a place.
 */
export const SHELL_BINDINGS: readonly ShellBinding[] = [
  binding("go.conversation", { key: "1", mod: true }, "Go to conversation"),
  binding("go.editor", { key: "2", mod: true }, "Go to editor"),
  binding("go.source-control", { key: "3", mod: true }, "Go to source control"),
  binding("toggle.taskRail", { key: "b", mod: true }, "Toggle task rail"),
  binding("toggle.inspector", { key: "i", mod: true, shift: true }, "Toggle inspector"),
  binding("toggle.utility", { key: "j", mod: true }, "Toggle utility panel"),
  binding("focus.composer", { key: "l", mod: true }, "Focus composer"),
  binding("open.palette", { key: "p", mod: true }, "Search everything"),
  binding("open.shortcuts", { key: "/", mod: true }, "Keyboard shortcuts"),
  binding("focus.next", { key: "F6" }, "Next region"),
  binding("focus.previous", { key: "F6", shift: true }, "Previous region"),
];

/**
 * What the shell offers a modal surface.
 *
 * Every other chord moves focus to a region, and a region behind a dialog is
 * somewhere the reader cannot see — the dialog would still be on screen with
 * the cursor lost behind it. Reaching the composer is the deliberate exception
 * in the other direction: the composer is *never* more than one key away, so
 * this closes whatever is open on the way there rather than refusing.
 */
const MODAL_SAFE: readonly string[] = ["open.palette", "open.shortcuts", "focus.composer"];

export function matchesChord(event: KeyboardEvent, chord: KeyChord): boolean {
  const modPressed = isApple ? event.metaKey : event.ctrlKey;
  if ((chord.mod ?? false) !== modPressed) return false;
  if ((chord.shift ?? false) !== event.shiftKey) return false;
  if ((chord.alt ?? false) !== event.altKey) return false;
  // The other modifier must not be held, or ⌘I and Ctrl+I would collide.
  const otherMod = isApple ? event.ctrlKey : event.metaKey;
  if (otherMod) return false;
  return event.key.toLowerCase() === chord.key.toLowerCase();
}

export function findBinding(event: KeyboardEvent): ShellBinding | undefined {
  return SHELL_BINDINGS.find((candidate) => matchesChord(event, candidate.chord));
}

export function bindingHint(id: string): string {
  return SHELL_BINDINGS.find((candidate) => candidate.id === id)?.hint ?? "";
}

export interface ShellKeymapHandlers {
  /** Opens the search-everything surface. */
  onPalette?: () => void;
  /** Opens the keyboard reference. */
  onShortcuts?: () => void;
  /**
   * Dismisses whatever modal surface is open.
   *
   * Called before focus is moved to the composer, so the one shortcut that is
   * promised to always reach it does not leave a dialog stranded on screen.
   */
  onDismissModals?: () => void;
}

/** Installs the shell keymap on `window` for as long as the frame is mounted. */
export function useShellKeymap(handlers: ShellKeymapHandlers = {}) {
  const { cycleRegion, dispatch, focusRegion, modalsRef } = useShell();
  // Read at event time rather than captured, so the listener does not need to
  // be torn down and rebuilt whenever a caller passes a fresh closure.
  const handlersRef = useRef(handlers);
  useEffect(() => {
    handlersRef.current = handlers;
  }, [handlers]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const match = findBinding(event);
      if (!match) return;

      // A dialog owns the keyboard while it is open. Anything that would move
      // focus to a region behind it is refused rather than obeyed.
      if (modalsRef.current > 0 && !MODAL_SAFE.includes(match.id)) return;

      if (match.id === "open.palette") {
        event.preventDefault();
        handlersRef.current.onPalette?.();
        return;
      }

      if (match.id === "open.shortcuts") {
        event.preventDefault();
        handlersRef.current.onShortcuts?.();
        return;
      }

      const destinationIndex = ["go.conversation", "go.editor", "go.source-control"].indexOf(
        match.id,
      );
      let action: ShellAction | null = null;

      if (destinationIndex >= 0) {
        action = { type: "navigate", destination: DESTINATIONS[destinationIndex] };
      } else if (match.id === "toggle.taskRail") {
        action = { type: "toggleTaskRail" };
      } else if (match.id === "toggle.inspector") {
        action = { type: "toggleInspector" };
      } else if (match.id === "toggle.utility") {
        action = { type: "toggleUtility" };
      }

      if (action) {
        event.preventDefault();
        dispatch(action);
        if (destinationIndex >= 0) {
          focusRegion(destinationIndex === 0 ? "conversation" : "workbench");
        }
        return;
      }

      if (match.id === "focus.composer") {
        event.preventDefault();
        // The composer is always one key away. If something is covering it,
        // that thing goes rather than the promise.
        if (modalsRef.current > 0) handlersRef.current.onDismissModals?.();
        focusRegion("composer");
        return;
      }

      if (match.id === "focus.next" || match.id === "focus.previous") {
        event.preventDefault();
        cycleRegion(match.id === "focus.next" ? 1 : -1);
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [cycleRegion, dispatch, focusRegion, modalsRef]);
}
