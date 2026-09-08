/**
 * The states every Mars surface can be in. Screens do not invent their own
 * vocabulary; they pick one of these and the shell renders it identically
 * everywhere — a rail row, a panel header, a tab, or a whole region.
 */
export type ActivityState =
  | "idle"
  | "queued"
  | "loading"
  | "streaming"
  | "blocked"
  | "failed"
  | "stale"
  | "done";

export const ACTIVITY_STATES: readonly ActivityState[] = [
  "idle",
  "queued",
  "loading",
  "streaming",
  "blocked",
  "failed",
  "stale",
  "done",
];

/**
 * `mark` and `pulse` — not hue — are what separate most states. Only failure
 * earns a second colour; everything live shares the one warm accent, and the
 * shape of the mark says whether it is moving, waiting on a human, or dead.
 */
export interface ActivityStatePresentation {
  readonly label: string;
  readonly tone: "accent" | "danger" | "muted" | "faint";
  readonly mark: "filled" | "ring" | "dashed" | "bar";
  readonly pulse: boolean;
  /** The run cannot advance until a human answers. */
  readonly needsHuman: boolean;
  /** What is on screen may not match the source of truth. */
  readonly needsRefresh: boolean;
}

export const ACTIVITY_STATE: Record<ActivityState, ActivityStatePresentation> = {
  idle: {
    label: "Idle",
    tone: "faint",
    mark: "ring",
    pulse: false,
    needsHuman: false,
    needsRefresh: false,
  },
  queued: {
    label: "Queued",
    tone: "faint",
    mark: "dashed",
    pulse: false,
    needsHuman: false,
    needsRefresh: false,
  },
  loading: {
    label: "Loading",
    tone: "muted",
    mark: "bar",
    pulse: true,
    needsHuman: false,
    needsRefresh: false,
  },
  streaming: {
    label: "Streaming",
    tone: "accent",
    mark: "filled",
    pulse: true,
    needsHuman: false,
    needsRefresh: false,
  },
  blocked: {
    label: "Blocked",
    tone: "accent",
    mark: "ring",
    pulse: false,
    needsHuman: true,
    needsRefresh: false,
  },
  failed: {
    label: "Failed",
    tone: "danger",
    mark: "filled",
    pulse: false,
    needsHuman: true,
    needsRefresh: false,
  },
  stale: {
    label: "Stale",
    tone: "muted",
    mark: "dashed",
    pulse: false,
    needsHuman: false,
    needsRefresh: true,
  },
  done: {
    label: "Done",
    tone: "muted",
    mark: "filled",
    pulse: false,
    needsHuman: false,
    needsRefresh: false,
  },
};

/** States that stop the work until someone acts, ordered most urgent first. */
export function attentionStates(states: readonly ActivityState[]): ActivityState[] {
  const rank: ActivityState[] = ["failed", "blocked", "stale"];
  return rank.filter((state) => states.includes(state));
}

/** The one state a group of rows should report upward. */
export function summarizeStates(states: readonly ActivityState[]): ActivityState {
  const order: ActivityState[] = [
    "failed",
    "blocked",
    "streaming",
    "loading",
    "stale",
    "queued",
    "done",
    "idle",
  ];
  return order.find((state) => states.includes(state)) ?? "idle";
}
