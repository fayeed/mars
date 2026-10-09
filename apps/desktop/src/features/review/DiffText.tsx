import { useMemo } from "react";

import { cn } from "@/lib/utils";
import { Mono, StateBlock } from "@/shell/primitives";

import { parseUnifiedDiff } from "./unified-diff";

export interface DiffTextProps {
  diff: string;
  /** Said when there is nothing to show. */
  emptyLabel?: string;
  onOpenFile?: (path: string, line: number | null) => void;
}

/**
 * A unified diff, read-only: file by file, hunk by hunk, with both sides'
 * line numbers. For a commit in history or a file's pending change, where
 * there is nothing to comment on — the task review keeps the full surface.
 */
export function DiffText({ diff, emptyLabel = "No changes", onOpenFile }: DiffTextProps) {
  const files = useMemo(() => parseUnifiedDiff(diff), [diff]);
  if (files.length === 0) {
    return <StateBlock state="idle" title={emptyLabel} />;
  }
  return (
    <div className="flex flex-col" data-slot="diff-text">
      {files.map((file) => (
        <section key={`${file.previousPath ?? ""}:${file.path}`} aria-label={file.path}>
          <header className="sticky top-0 z-10 flex items-center gap-2 border-b border-border bg-surface px-2 py-1">
            <button
              type="button"
              className="min-w-0 truncate text-left font-mono text-[11px] text-foreground outline-none hover:underline focus-visible:underline"
              onClick={() => onOpenFile?.(file.path, file.hunks[0]?.newStart ?? null)}
            >
              {file.previousPath && file.previousPath !== file.path
                ? `${file.previousPath} → ${file.path}`
                : file.path}
            </button>
            <Mono className="ml-auto shrink-0 text-faint">
              +{file.added} −{file.removed}
            </Mono>
          </header>
          {file.binary ? (
            <p className="px-2 py-1 text-[11px] text-muted-foreground">Binary file; no lines to show.</p>
          ) : (
            file.hunks.map((hunk) => (
              <div key={hunk.header} className="border-b border-border">
                <Mono className="block bg-sunken px-2 py-0.5 text-faint">{hunk.header}</Mono>
                <table className="w-full border-collapse font-mono text-[11px] leading-[1.45]">
                  <tbody>
                    {hunk.lines.map((line, index) => (
                      <tr
                        key={index}
                        className={cn(
                          line.kind === "added" && "bg-[color-mix(in_oklch,var(--primary)_10%,transparent)]",
                          line.kind === "removed" && "bg-[color-mix(in_oklch,var(--destructive)_10%,transparent)]",
                        )}
                      >
                        <td className="w-10 select-none pr-1 text-right align-top text-faint">
                          {line.oldLine ?? ""}
                        </td>
                        <td className="w-10 select-none pr-2 text-right align-top text-faint">
                          {line.newLine ?? ""}
                        </td>
                        <td className="w-3 select-none align-top text-faint">
                          {line.kind === "added" ? "+" : line.kind === "removed" ? "−" : " "}
                        </td>
                        <td className="whitespace-pre-wrap break-all pr-2 align-top text-foreground">
                          {line.text}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ))
          )}
        </section>
      ))}
    </div>
  );
}
