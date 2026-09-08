/**
 * The Mars shell and its design system.
 *
 * Screens compose these regions and primitives; they do not invent chrome,
 * empty states, or state vocabulary of their own.
 */
export {
  ACTIVITY_STATE,
  ACTIVITY_STATES,
  attentionStates,
  summarizeStates,
  type ActivityState,
  type ActivityStatePresentation,
} from "@/shell/activity-state";
export { AppRail } from "@/shell/AppRail";
export { Composer, type ComposerProps } from "@/shell/Composer";
export {
  applyCompletion,
  filterCandidates,
  readTrigger,
  type ComposerTrigger,
  type TriggerKind,
} from "@/shell/composer-input";
export {
  ConversationColumn,
  type ConversationColumnProps,
} from "@/shell/ConversationColumn";
export { Inspector, type InspectorProps } from "@/shell/Inspector";
export {
  bindingHint,
  chordHint,
  findBinding,
  matchesChord,
  SHELL_BINDINGS,
  useShellKeymap,
  type KeyChord,
  type ShellBinding,
} from "@/shell/keymap";
export type {
  ComposerAgent,
  ComposerAttachment,
  ComposerSkill,
  ComposerSubmission,
  InspectorContent,
  InspectorStatuses,
  MentionTarget,
  ShellTask,
  TabStatus,
  UtilityContent,
  UtilityStatuses,
} from "@/shell/model";
export {
  CollapsedStrip,
  KeyHint,
  MetaLabel,
  Mono,
  PanelHeader,
  RailButton,
  StateBlock,
  StateDot,
  StateLabel,
} from "@/shell/primitives";
export { Region } from "@/shell/Region";
export {
  anchoredScrollTop,
  isPinnedToBottom,
  useScrollAnchor,
  type ScrollMetrics,
} from "@/shell/scroll-anchor";
export {
  ShellFrame,
  type ShellFrameProps,
  type WorkbenchSurface,
  type WorkbenchSurfaces,
} from "@/shell/ShellFrame";
export {
  DESTINATIONS,
  INITIAL_LAYOUT,
  INSPECTOR_TABS,
  readStoredLayout,
  REGION_ORDER,
  shellReducer,
  ShellProvider,
  useShell,
  UTILITY_TABS,
  type Destination,
  type InspectorTab,
  type ShellAction,
  type ShellLayout,
  type ShellRegion,
  type UtilityTab,
} from "@/shell/shell-store";
export { StatusBar, type StatusBarProps } from "@/shell/StatusBar";
export { TaskRail, type TaskRailProps } from "@/shell/TaskRail";
export {
  nextThemeMode,
  readStoredTheme,
  resolveTheme,
  ThemeProvider,
  useTheme,
  type ThemeMode,
} from "@/shell/theme";
export {
  UtilityBody,
  UtilityTabStrip,
  type UtilityPanelProps,
} from "@/shell/UtilityPanel";
