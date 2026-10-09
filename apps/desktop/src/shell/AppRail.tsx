import {
  BotIcon,
  BrainIcon,
  CircleUserIcon,
  FileCode2Icon,
  GitCompareIcon,
  MessageSquareIcon,
  MonitorIcon,
  MoonIcon,
  SunIcon,
} from "lucide-react";
import type { ComponentType } from "react";

import { RailButton } from "@/shell/primitives";
import { bindingHint } from "@/shell/keymap";
import { DESTINATIONS, useShell, type Destination } from "@/shell/shell-store";
import { useTheme, type ThemeMode } from "@/shell/theme";

const destinationMeta: Record<
  Destination,
  { icon: ComponentType<{ className?: string }>; label: string; binding: string }
> = {
  conversation: { icon: MessageSquareIcon, label: "Conversation", binding: "go.conversation" },
  editor: { icon: FileCode2Icon, label: "Editor", binding: "go.editor" },
  "source-control": { icon: GitCompareIcon, label: "Source control", binding: "go.source-control" },
  agents: { icon: BotIcon, label: "Agents", binding: "go.agents" },
  knowledge: { icon: BrainIcon, label: "Knowledge", binding: "go.knowledge" },
};

const themeMeta: Record<ThemeMode, { icon: ComponentType<{ className?: string }>; label: string }> =
  {
    dark: { icon: MoonIcon, label: "Theme: dark" },
    light: { icon: SunIcon, label: "Theme: light" },
    system: { icon: MonitorIcon, label: "Theme: match system" },
  };

/**
 * The destinations, then theme and account. Nothing else is ever added here:
 * the rail says where you are, not what you can do.
 */
export function AppRail({
  accountLabel = "Account",
  badges = {},
  onAccount,
}: {
  accountLabel?: string;
  /**
   * How many things at a destination are waiting on the person — a decision
   * to review, a memory to approve. A count, not a feature: it says where
   * attention is owed.
   */
  badges?: Partial<Record<Destination, number>>;
  /** Opens the management surfaces, which are not destinations. */
  onAccount?: () => void;
}) {
  const { dispatch, focusRegion, layout } = useShell();
  const theme = useTheme();
  const ThemeIcon = themeMeta[theme.mode].icon;

  return (
    <nav
      aria-label="Destinations"
      className="flex w-[var(--shell-rail-width)] shrink-0 flex-col items-center border-r border-border bg-surface"
    >
      {DESTINATIONS.map((destination) => {
        const meta = destinationMeta[destination];
        const Icon = meta.icon;
        const current = layout.destination === destination;
        const waiting = badges[destination] ?? 0;
        const label = waiting > 0 ? `${meta.label}, ${waiting} waiting on you` : meta.label;
        return (
          <RailButton
            key={destination}
            aria-current={current ? "page" : undefined}
            title={`${label} · ${bindingHint(meta.binding)}`}
            onClick={() => {
              dispatch({ type: "navigate", destination });
              focusRegion(destination === "conversation" ? "conversation" : "workbench");
            }}
          >
            <Icon />
            {waiting > 0 ? (
              <span
                aria-hidden="true"
                data-slot="rail-badge"
                className="absolute top-1.5 right-1.5 min-w-3.5 rounded-[2px] bg-primary px-0.5 text-center font-mono text-[9px] leading-3.5 text-primary-foreground"
              >
                {waiting > 99 ? "99+" : waiting}
              </span>
            ) : null}
            <span className="sr-only">{label}</span>
          </RailButton>
        );
      })}

      <div className="flex-1" />

      <RailButton marker="none" title={themeMeta[theme.mode].label} onClick={theme.toggle}>
        <ThemeIcon />
        <span className="sr-only">{themeMeta[theme.mode].label}</span>
      </RailButton>
      <RailButton marker="none" title={accountLabel} onClick={onAccount}>
        <CircleUserIcon />
        <span className="sr-only">{accountLabel}</span>
      </RailButton>
    </nav>
  );
}
