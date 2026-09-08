import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  type ReactNode,
  type RefObject,
} from "react";

/**
 * Three destinations. Not a toolbar of every feature — a set of places the
 * product can be. Everything else is a panel inside one of them.
 */
export type Destination = "conversation" | "editor" | "source-control";

export const DESTINATIONS: readonly Destination[] = ["conversation", "editor", "source-control"];

export type InspectorTab = "context" | "plan" | "diff" | "brain";
export type UtilityTab = "problems" | "tests" | "checks" | "terminal";

export const INSPECTOR_TABS: readonly InspectorTab[] = ["context", "plan", "diff", "brain"];
export const UTILITY_TABS: readonly UtilityTab[] = ["problems", "tests", "checks", "terminal"];

/** Focusable regions, in the order F6 walks them. */
export type ShellRegion =
  | "task-rail"
  | "workbench"
  | "conversation"
  | "composer"
  | "inspector"
  | "utility";

export const REGION_ORDER: readonly ShellRegion[] = [
  "task-rail",
  "workbench",
  "conversation",
  "composer",
  "inspector",
  "utility",
];

export interface ShellLayout {
  destination: Destination;
  taskRailCollapsed: boolean;
  inspectorCollapsed: boolean;
  inspectorTab: InspectorTab;
  utilityOpen: boolean;
  utilityTab: UtilityTab;
}

export const INITIAL_LAYOUT: ShellLayout = {
  destination: "conversation",
  taskRailCollapsed: false,
  inspectorCollapsed: false,
  inspectorTab: "context",
  utilityOpen: false,
  utilityTab: "problems",
};

export type ShellAction =
  | { type: "navigate"; destination: Destination }
  | { type: "toggleTaskRail" }
  | { type: "setTaskRail"; collapsed: boolean }
  | { type: "toggleInspector" }
  | { type: "openInspector"; tab?: InspectorTab }
  | { type: "activateInspectorTab"; tab: InspectorTab }
  | { type: "toggleUtility" }
  | { type: "openUtility"; tab?: UtilityTab }
  | { type: "activateUtilityTab"; tab: UtilityTab };

/**
 * The whole shell's layout logic lives here so it can be reasoned about and
 * tested without a DOM. Two rules it enforces:
 *
 *  - Nothing but an explicit user action changes a region's visibility.
 *    Navigating between destinations never collapses the inspector or closes
 *    the utility panel, so the conversation never loses width by surprise.
 *  - Asking for a tab is asking for the panel: a tab request opens the panel
 *    it belongs to.
 */
export function shellReducer(state: ShellLayout, action: ShellAction): ShellLayout {
  switch (action.type) {
    case "navigate":
      return state.destination === action.destination
        ? state
        : { ...state, destination: action.destination };
    case "toggleTaskRail":
      return { ...state, taskRailCollapsed: !state.taskRailCollapsed };
    case "setTaskRail":
      return { ...state, taskRailCollapsed: action.collapsed };
    case "toggleInspector":
      return { ...state, inspectorCollapsed: !state.inspectorCollapsed };
    case "openInspector":
      return {
        ...state,
        inspectorCollapsed: false,
        inspectorTab: action.tab ?? state.inspectorTab,
      };
    case "activateInspectorTab":
      // Re-picking the open tab from the expanded panel is a no-op, but picking
      // any tab from the collapsed rail expands it.
      return { ...state, inspectorCollapsed: false, inspectorTab: action.tab };
    case "toggleUtility":
      return { ...state, utilityOpen: !state.utilityOpen };
    case "openUtility":
      return { ...state, utilityOpen: true, utilityTab: action.tab ?? state.utilityTab };
    case "activateUtilityTab":
      // Clicking the tab that is already showing closes the panel again; that
      // is the only way a tab click can take the panel away.
      if (state.utilityOpen && state.utilityTab === action.tab) {
        return { ...state, utilityOpen: false };
      }
      return { ...state, utilityOpen: true, utilityTab: action.tab };
    default:
      return state;
  }
}

const LAYOUT_STORAGE_KEY = "mars.shell.layout";

export function readStoredLayout(storage: Pick<Storage, "getItem"> | undefined): ShellLayout {
  if (!storage) return INITIAL_LAYOUT;
  try {
    const raw = storage.getItem(LAYOUT_STORAGE_KEY);
    if (!raw) return INITIAL_LAYOUT;
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return INITIAL_LAYOUT;
    const candidate = parsed as Partial<ShellLayout>;
    return {
      destination: DESTINATIONS.includes(candidate.destination as Destination)
        ? (candidate.destination as Destination)
        : INITIAL_LAYOUT.destination,
      taskRailCollapsed: candidate.taskRailCollapsed === true,
      inspectorCollapsed: candidate.inspectorCollapsed === true,
      inspectorTab: INSPECTOR_TABS.includes(candidate.inspectorTab as InspectorTab)
        ? (candidate.inspectorTab as InspectorTab)
        : INITIAL_LAYOUT.inspectorTab,
      utilityOpen: candidate.utilityOpen === true,
      utilityTab: UTILITY_TABS.includes(candidate.utilityTab as UtilityTab)
        ? (candidate.utilityTab as UtilityTab)
        : INITIAL_LAYOUT.utilityTab,
    };
  } catch {
    return INITIAL_LAYOUT;
  }
}

interface ShellContextValue {
  layout: ShellLayout;
  dispatch: (action: ShellAction) => void;
  registerRegion: (region: ShellRegion, element: HTMLElement | null) => void;
  focusRegion: (region: ShellRegion) => boolean;
  cycleRegion: (direction: 1 | -1) => void;
  regionsRef: RefObject<Partial<Record<ShellRegion, HTMLElement | null>>>;
}

const ShellContext = createContext<ShellContextValue | null>(null);

export function ShellProvider({
  children,
  initialLayout,
  storage,
}: {
  children: ReactNode;
  initialLayout?: Partial<ShellLayout>;
  storage?: Pick<Storage, "getItem" | "setItem">;
}) {
  const store = storage ?? (typeof window === "undefined" ? undefined : window.localStorage);
  const [layout, dispatch] = useReducer(shellReducer, undefined, () => ({
    ...readStoredLayout(store),
    ...initialLayout,
  }));
  const regionsRef = useRef<Partial<Record<ShellRegion, HTMLElement | null>>>({});
  const lastRegionRef = useRef<ShellRegion>("conversation");
  // A region asked for before it exists — navigating to the editor focuses the
  // workbench that the same click is about to mount.
  const pendingFocusRef = useRef<ShellRegion | null>(null);

  useEffect(() => {
    try {
      store?.setItem(LAYOUT_STORAGE_KEY, JSON.stringify(layout));
    } catch {
      // A profile with storage disabled still gets a working shell; it just
      // starts from defaults every launch.
    }
  }, [layout, store]);

  const registerRegion = useCallback((region: ShellRegion, element: HTMLElement | null) => {
    regionsRef.current[region] = element;
    if (element && pendingFocusRef.current === region) {
      pendingFocusRef.current = null;
      lastRegionRef.current = region;
      element.focus();
    }
  }, []);

  const focusRegion = useCallback((region: ShellRegion) => {
    const element = regionsRef.current[region];
    if (!element) {
      pendingFocusRef.current = region;
      return false;
    }
    pendingFocusRef.current = null;
    element.focus();
    lastRegionRef.current = region;
    return true;
  }, []);

  const cycleRegion = useCallback(
    (direction: 1 | -1) => {
      const available = REGION_ORDER.filter((region) => regionsRef.current[region]);
      if (available.length === 0) return;
      const current = available.indexOf(lastRegionRef.current);
      const next = available[(current + direction + available.length) % available.length];
      focusRegion(next);
    },
    [focusRegion],
  );

  const value = useMemo<ShellContextValue>(
    () => ({ layout, dispatch, registerRegion, focusRegion, cycleRegion, regionsRef }),
    [cycleRegion, focusRegion, layout, registerRegion],
  );

  return <ShellContext value={value}>{children}</ShellContext>;
}

export function useShell(): ShellContextValue {
  const value = useContext(ShellContext);
  if (!value) throw new Error("useShell must be used inside <ShellProvider>");
  return value;
}
