import { useEffect } from "react";

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
  binding("focus.next", { key: "F6" }, "Next region"),
  binding("focus.previous", { key: "F6", shift: true }, "Previous region"),
];

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

/** Installs the shell keymap on `window` for as long as the frame is mounted. */
export function useShellKeymap() {
  const { cycleRegion, dispatch, focusRegion } = useShell();

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const match = findBinding(event);
      if (!match) return;

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
  }, [cycleRegion, dispatch, focusRegion]);
}
