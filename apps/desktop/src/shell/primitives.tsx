import { cva, type VariantProps } from "class-variance-authority";
import type { ComponentProps, ReactNode } from "react";

import { cn } from "@/lib/utils";
import {
  ACTIVITY_STATE,
  type ActivityState,
} from "@/shell/activity-state";

const toneText: Record<ActivityStatePresentationTone, string> = {
  accent: "text-primary",
  danger: "text-destructive",
  muted: "text-muted-foreground",
  faint: "text-faint",
};

type ActivityStatePresentationTone = "accent" | "danger" | "muted" | "faint";

const markClass: Record<string, string> = {
  filled: "bg-current",
  ring: "border border-current bg-transparent",
  dashed: "border border-dashed border-current bg-transparent",
  bar: "h-px w-2.5 bg-current",
};

/**
 * The single state mark used across rails, headers, tabs and rows. Shape
 * carries the meaning so the accent stays reserved and colour-blind readers
 * are not asked to tell amber from amber.
 */
export function StateDot({
  className,
  state,
  ...props
}: ComponentProps<"span"> & { state: ActivityState }) {
  const presentation = ACTIVITY_STATE[state];
  return (
    <span
      aria-hidden="true"
      data-state={state}
      data-slot="state-dot"
      className={cn(
        "inline-block size-1.5 shrink-0 rounded-[1px]",
        toneText[presentation.tone],
        markClass[presentation.mark],
        presentation.pulse && "shell-pulse",
        className,
      )}
      {...props}
    />
  );
}

export function StateLabel({
  className,
  detail,
  state,
  ...props
}: ComponentProps<"span"> & { state: ActivityState; detail?: string }) {
  const presentation = ACTIVITY_STATE[state];
  return (
    <span
      className={cn("inline-flex items-center gap-1.5 text-[11px]", className)}
      data-slot="state-label"
      {...props}
    >
      <StateDot state={state} />
      <span className={toneText[presentation.tone]}>{presentation.label}</span>
      {detail ? <Mono className="text-faint">{detail}</Mono> : null}
    </span>
  );
}

/** Machine-authored text: paths, shas, commands, timings, counts. */
export function Mono({ className, ...props }: ComponentProps<"span">) {
  return (
    <span
      data-slot="mono"
      className={cn("font-mono text-[11px] tabular-nums", className)}
      {...props}
    />
  );
}

/** The small uppercase label that names a region or a group. */
export function MetaLabel({ className, ...props }: ComponentProps<"span">) {
  return <span data-slot="meta-label" className={cn("shell-label", className)} {...props} />;
}

export function KeyHint({ className, keys, ...props }: ComponentProps<"kbd"> & { keys: string }) {
  return (
    <kbd
      data-slot="key-hint"
      className={cn(
        "rounded-sm border border-border px-1 py-px text-[10px] leading-4 text-faint",
        className,
      )}
      {...props}
    >
      {keys}
    </kbd>
  );
}

/**
 * Every panel in the shell wears the same 34px header: name on the left,
 * state and actions on the right, one hairline beneath.
 */
export function PanelHeader({
  actions,
  children,
  className,
  state,
  title,
  ...props
}: Omit<ComponentProps<"div">, "title"> & {
  actions?: ReactNode;
  state?: ActivityState;
  title: ReactNode;
}) {
  return (
    <div
      data-slot="panel-header"
      className={cn(
        "flex h-[var(--shell-header-height)] shrink-0 items-center gap-2 border-b border-border bg-surface px-2",
        className,
      )}
      {...props}
    >
      <MetaLabel className="truncate text-foreground">{title}</MetaLabel>
      {state ? <StateDot state={state} /> : null}
      <div className="min-w-0 flex-1">{children}</div>
      {actions ? <div className="flex shrink-0 items-center gap-0.5">{actions}</div> : null}
    </div>
  );
}

const railButtonVariants = cva(
  "relative flex items-center justify-center text-muted-foreground transition-colors outline-none hover:text-foreground focus-visible:text-foreground focus-visible:shadow-[inset_0_0_0_1px_var(--ring)] disabled:pointer-events-none disabled:text-faint aria-[current=page]:text-primary [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      /** The 2px warm marker that says "you are here". */
      marker: {
        left: "before:absolute before:inset-y-1.5 before:left-0 before:w-0.5 before:bg-transparent aria-[current=page]:before:bg-primary",
        right:
          "before:absolute before:inset-y-1.5 before:right-0 before:w-0.5 before:bg-transparent aria-[current=page]:before:bg-primary",
        bottom:
          "before:absolute before:inset-x-1.5 before:bottom-0 before:h-0.5 before:bg-transparent aria-[current=page]:before:bg-primary",
        none: "",
      },
      size: {
        rail: "size-[var(--shell-rail-width)]",
        strip: "h-8 w-[var(--shell-strip-width)]",
        tab: "h-[var(--shell-header-height)] gap-1.5 px-2 text-[11px]",
      },
    },
    defaultVariants: { marker: "left", size: "rail" },
  },
);

export function RailButton({
  className,
  marker,
  size,
  ...props
}: ComponentProps<"button"> & VariantProps<typeof railButtonVariants>) {
  return (
    <button
      type="button"
      data-slot="rail-button"
      className={cn(railButtonVariants({ marker, size, className }))}
      {...props}
    />
  );
}

/**
 * What a region shows instead of content: loading, blocked, failed, stale, or
 * simply empty. One component so no screen hand-rolls its own empty state.
 */
export function StateBlock({
  action,
  className,
  description,
  state,
  title,
  ...props
}: Omit<ComponentProps<"div">, "title"> & {
  action?: ReactNode;
  description?: ReactNode;
  state: ActivityState;
  title: ReactNode;
}) {
  const presentation = ACTIVITY_STATE[state];
  return (
    <div
      data-slot="state-block"
      data-state={state}
      role={presentation.tone === "danger" ? "alert" : "status"}
      className={cn(
        "flex h-full min-h-0 flex-col items-center justify-center gap-2 px-6 py-8 text-center",
        className,
      )}
      {...props}
    >
      <StateLabel state={state} />
      <p className="text-[13px] text-foreground">{title}</p>
      {description ? (
        <p className="max-w-80 text-[11px] leading-relaxed text-muted-foreground">{description}</p>
      ) : null}
      {action ? <div className="pt-1">{action}</div> : null}
    </div>
  );
}

/** A collapsed rail: a labelled vertical strip, still clickable, still stateful. */
export function CollapsedStrip({
  children,
  className,
  label,
  onExpand,
  side,
  state,
  ...props
}: Omit<ComponentProps<"div">, "children"> & {
  children?: ReactNode;
  label: string;
  onExpand: () => void;
  side: "left" | "right";
  state?: ActivityState;
}) {
  return (
    <div
      data-slot="collapsed-strip"
      className={cn(
        "flex w-[var(--shell-strip-width)] shrink-0 flex-col items-center gap-2 bg-surface py-2",
        side === "left" ? "border-r border-border" : "border-l border-border",
        className,
      )}
      {...props}
    >
      <button
        type="button"
        aria-expanded={false}
        className="flex flex-1 flex-col items-center gap-2 text-muted-foreground outline-none hover:text-foreground focus-visible:text-foreground focus-visible:shadow-[inset_0_0_0_1px_var(--ring)]"
        onClick={onExpand}
      >
        {state ? <StateDot state={state} /> : null}
        <span className="shell-strip-label shell-label text-current">{label}</span>
      </button>
      {children}
    </div>
  );
}
