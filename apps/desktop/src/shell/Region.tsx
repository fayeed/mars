import { useCallback, type ComponentProps } from "react";

import { cn } from "@/lib/utils";
import { useShell, type ShellRegion } from "@/shell/shell-store";

/**
 * A shell region: a landmark that F6 can reach, that takes focus without
 * trapping it, and that shows a hairline when it holds focus.
 */
export function Region({
  as = "section",
  children,
  className,
  label,
  region,
  ...props
}: Omit<ComponentProps<"section">, "ref"> & {
  as?: "section" | "aside" | "div" | "nav";
  label: string;
  region: ShellRegion;
}) {
  const { registerRegion } = useShell();
  // The tag varies for landmark semantics; the DOM type does not.
  const Element = as as "div";

  const ref = useCallback(
    (element: HTMLDivElement | null) => {
      registerRegion(region, element);
      return () => registerRegion(region, null);
    },
    [region, registerRegion],
  );

  return (
    <Element
      ref={ref}
      aria-label={label}
      data-region={region}
      tabIndex={-1}
      className={cn("shell-focusable flex min-h-0 min-w-0 flex-col", className)}
      {...props}
    >
      {children}
    </Element>
  );
}
