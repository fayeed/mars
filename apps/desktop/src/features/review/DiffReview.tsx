import { FileDiffIcon, MessageSquarePlusIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { cn } from "@/lib/utils";
import { languageOf, type DiffFile, type DiffLine } from "@/features/review/unified-diff";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, PanelHeader, StateBlock } from "@/shell/primitives";

export interface DiffReviewProps {
  files: DiffFile[];
  state: ActivityState;
  error?: string | null;
  branchName?: string;
  headOid?: string | null;
  /**
   * Posts a line comment into the conversation.
   *
   * `side` says which file the line number belongs to. A removed line and an
   * added line can carry the same number and mean different things, so a
   * comment that did not say which would be unreadable a day later.
   */
  onComment: (path: string, line: number, text: string, side: "old" | "new") => void;
  onOpenFile?: (path: string, line: number | null) => void;
  /** Swapped in tests; the real one lazily pulls Monaco's tokenizer. */
  colorize?: (code: string, language: string) => Promise<string[]>;
}

const CHANGE_LABEL: Record<DiffFile["change"], string> = {
  added: "added",
  removed: "removed",
  modified: "modified",
  renamed: "renamed",
};

/**
 * Syntax highlighting for a block of diff text.
 *
 * Monaco's `colorize` is the tokenizer without the editor: it takes source and
 * returns HTML. Colouring a whole hunk at once rather than line by line keeps
 * the tokenizer's state across lines, so a multi-line string or comment is not
 * restarted on every row.
 */
async function monacoColorize(code: string, language: string): Promise<string[]> {
  const monaco = await import("monaco-editor");
  const html = await monaco.editor.colorize(code, language, { tabSize: 2 });
  return html.split(/<br\s*\/?>/);
}

/**
 * One diff line.
 *
 * The HTML comes from Monaco's own tokenizer over source that is already on
 * this machine, in a repository the person trusted, and Monaco escapes the text
 * it colours. When there is no highlighting the text is rendered as text.
 */
function LineText({ html, text }: { html: string | undefined; text: string }) {
  if (html === undefined) {
    return <span className="whitespace-pre">{text === "" ? " " : text}</span>;
  }
  return <span className="whitespace-pre" dangerouslySetInnerHTML={{ __html: html }} />;
}

function FileList({
  files,
  onSelect,
  selected,
}: {
  files: DiffFile[];
  onSelect: (path: string) => void;
  selected: string | null;
}) {
  return (
    <ul className="min-h-0 flex-1 overflow-y-auto" aria-label="Changed files">
      {files.map((file) => (
        <li key={file.path}>
          <button
            type="button"
            aria-current={file.path === selected ? "true" : undefined}
            onClick={() => onSelect(file.path)}
            className={cn(
              "relative flex w-full flex-col gap-0.5 border-b border-border px-2 py-1.5 text-left outline-none",
              "before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-transparent",
              "hover:bg-surface-raised focus-visible:shadow-[inset_0_0_0_1px_var(--ring)]",
              file.path === selected && "bg-surface-raised before:bg-primary",
            )}
          >
            <Mono className="min-w-0 truncate text-foreground">
              {file.path.split("/").at(-1)}
            </Mono>
            <span className="flex items-baseline gap-2">
              <Mono className="min-w-0 flex-1 truncate text-faint">
                {file.path.split("/").slice(0, -1).join("/")}
              </Mono>
              {file.binary ? (
                <Mono className="shrink-0 text-faint">binary</Mono>
              ) : (
                <>
                  <Mono className="shrink-0 text-primary">+{file.added}</Mono>
                  <Mono className="shrink-0 text-destructive">−{file.removed}</Mono>
                </>
              )}
            </span>
            {file.change !== "modified" ? (
              <Mono className="text-faint">
                {CHANGE_LABEL[file.change]}
                {file.previousPath ? ` from ${file.previousPath}` : ""}
              </Mono>
            ) : null}
          </button>
        </li>
      ))}
    </ul>
  );
}

/**
 * Code beside the conversation, not instead of it.
 *
 * This is the screen that has to prove the shell's central rule, so it does
 * nothing to earn its space: it opens in the workbench, the conversation keeps
 * its column, and the composer stays live. A comment on a line is posted into
 * the conversation rather than kept here, because a review remark is part of
 * the record of the task, not an annotation floating beside it.
 */
export function DiffReview({
  branchName,
  colorize = monacoColorize,
  error,
  files,
  headOid,
  onComment,
  onOpenFile,
  state,
}: DiffReviewProps) {
  const [selected, setSelected] = useState<string | null>(files[0]?.path ?? null);
  // Keyed by row, not by line number: two rows can carry the same number.
  const [commentAt, setCommentAt] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [highlighted, setHighlighted] = useState<Map<string, string[]>>(new Map());
  const requested = useRef(new Set<string>());

  // A selection that is no longer in the diff falls back to the first file
  // rather than being repaired in an effect, so there is no render where the
  // panel is pointed at a file that is not there.
  const file = files.find((candidate) => candidate.path === selected) ?? files[0] ?? null;

  const language = file ? languageOf(file.path) : null;

  useEffect(() => {
    if (!file || !language || file.binary) return;
    let active = true;
    for (const [index, hunk] of file.hunks.entries()) {
      const key = `${file.path}:${index}`;
      if (requested.current.has(key)) continue;
      requested.current.add(key);
      colorize(hunk.lines.map((line) => line.text).join("\n"), language)
        .then((lines) => {
          if (!active) return;
          setHighlighted((current) => new Map(current).set(key, lines));
        })
        // Highlighting is an enhancement. Losing it costs colour, not content.
        .catch(() => undefined);
    }
    return () => {
      active = false;
    };
  }, [colorize, file, language]);

  const post = useCallback(
    (line: number, side: "old" | "new") => {
      const text = draft.trim();
      if (!text || !file) return;
      onComment(file.path, line, text, side);
      setDraft("");
      setCommentAt(null);
    },
    [draft, file, onComment],
  );

  const totals = useMemo(
    () =>
      files.reduce(
        (sum, candidate) => ({
          added: sum.added + candidate.added,
          removed: sum.removed + candidate.removed,
        }),
        { added: 0, removed: 0 },
      ),
    [files],
  );

  if (state === "loading") {
    return <StateBlock state="loading" title="Reading the worktree" />;
  }
  if (state === "failed") {
    return (
      <StateBlock
        state="failed"
        title="The diff could not be read"
        description={error ?? "Nothing was changed. The worktree is as it was."}
      />
    );
  }
  if (files.length === 0) {
    return (
      <StateBlock
        state="idle"
        title="No changes to review"
        description="Nothing in this worktree differs from its base."
      />
    );
  }

  return (
    <div className="flex h-full min-h-0">
      <div className="flex w-64 shrink-0 flex-col border-r border-border">
        <PanelHeader
          title="Changes"
          actions={
            <>
              <Mono className="text-primary">+{totals.added}</Mono>
              <Mono className="text-destructive">−{totals.removed}</Mono>
            </>
          }
        />
        <FileList files={files} onSelect={setSelected} selected={file?.path ?? null} />
      </div>

      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <PanelHeader
          title={
            <span className="flex items-baseline gap-2">
              <FileDiffIcon className="size-3.5 shrink-0 self-center text-faint" />
              <Mono className="truncate text-foreground">{file?.path}</Mono>
            </span>
          }
          actions={
            <>
              {branchName ? <Mono className="text-faint">{branchName}</Mono> : null}
              {headOid ? <Mono className="text-faint">{headOid.slice(0, 7)}</Mono> : null}
            </>
          }
        />

        <div className="min-h-0 flex-1 overflow-auto">
          {!file ? null : file.binary ? (
            <StateBlock
              state="idle"
              title="Binary file"
              description="Git recorded that this file changed but not how. There is nothing to show line by line."
            />
          ) : file.hunks.length === 0 ? (
            <StateBlock
              state="idle"
              title="New file, not yet staged"
              description="Git has not seen this file, so it has no diff against the base. Open it to read it."
              action={
                onOpenFile ? (
                  <button
                    type="button"
                    onClick={() => onOpenFile(file.path, null)}
                    className="border border-border px-2 py-1 text-[11px] hover:border-border-strong"
                  >
                    Open {file.path.split("/").at(-1)}
                  </button>
                ) : null
              }
            />
          ) : (
            file.hunks.map((hunk, hunkIndex) => {
              const lines = highlighted.get(`${file.path}:${hunkIndex}`);
              return (
                <div key={hunk.header}>
                  <div className="sticky top-0 z-10 border-y border-border bg-surface px-2 py-0.5">
                    <Mono className="text-faint">{hunk.header}</Mono>
                  </div>
                  <table className="w-full border-collapse">
                    <tbody>
                      {hunk.lines.map((line, lineIndex) => {
                        const rowKey = `${hunkIndex}:${lineIndex}`;
                        return (
                          <DiffRow
                            commentOpen={commentAt === rowKey}
                            draft={draft}
                            html={lines?.[lineIndex]}
                            key={rowKey}
                            line={line}
                            onCancel={() => setCommentAt(null)}
                            onDraft={setDraft}
                            onOpenComment={() => {
                              setCommentAt(rowKey);
                              setDraft("");
                            }}
                            onPost={() =>
                              post(
                                line.newLine ?? line.oldLine ?? 0,
                                line.newLine === null ? "old" : "new",
                              )
                            }
                          />
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}

const ROW_TONE: Record<DiffLine["kind"], string> = {
  added: "bg-primary/8",
  removed: "bg-destructive/8",
  context: "",
};

const MARK: Record<DiffLine["kind"], string> = {
  added: "+",
  removed: "−",
  context: " ",
};

function DiffRow({
  commentOpen,
  draft,
  html,
  line,
  onCancel,
  onDraft,
  onOpenComment,
  onPost,
}: {
  commentOpen: boolean;
  draft: string;
  html: string | undefined;
  line: DiffLine;
  onCancel: () => void;
  onDraft: (value: string) => void;
  onOpenComment: () => void;
  onPost: () => void;
}) {
  const anchor = line.newLine ?? line.oldLine;
  // A removed line's number refers to the file as it was, and the label says
  // so rather than leaving two buttons both claiming "line 81".
  const label =
    line.newLine === null ? `line ${anchor} before the change` : `line ${anchor}`;
  return (
    <>
      <tr className={cn("group align-baseline", ROW_TONE[line.kind])}>
        <td className="w-10 select-none border-r border-border px-1 text-right align-top">
          <Mono className="text-faint">{line.oldLine ?? ""}</Mono>
        </td>
        <td className="w-10 select-none border-r border-border px-1 text-right align-top">
          <Mono className="text-faint">{line.newLine ?? ""}</Mono>
        </td>
        <td className="w-5 select-none text-center align-top">
          <Mono
            className={
              line.kind === "added"
                ? "text-primary"
                : line.kind === "removed"
                  ? "text-destructive"
                  : "text-faint"
            }
          >
            {MARK[line.kind]}
          </Mono>
        </td>
        <td className="w-full py-px pr-6 align-top font-mono text-[11px] leading-relaxed">
          <LineText html={html} text={line.text} />
        </td>
        <td className="w-6 align-top">
          {anchor === null ? null : (
            <button
              type="button"
              aria-label={`Comment on ${label}`}
              onClick={onOpenComment}
              className="opacity-0 outline-none group-hover:opacity-100 focus-visible:opacity-100"
            >
              <MessageSquarePlusIcon className="size-3 text-faint hover:text-foreground" />
            </button>
          )}
        </td>
      </tr>
      {commentOpen && anchor !== null ? (
        <tr>
          <td colSpan={5} className="border-y border-border bg-surface px-2 py-1.5">
            <div className="flex flex-col gap-1">
              <MetaLabel>Comment on {label}</MetaLabel>
              <div className="flex items-center gap-1">
                <input
                  aria-label={`Comment on ${label}`}
                  autoFocus
                  className="min-w-0 flex-1 border border-border bg-transparent px-1.5 py-0.5 text-[12px] text-foreground outline-none focus-visible:border-border-strong"
                  onChange={(changed) => onDraft(changed.target.value)}
                  onKeyDown={(pressed) => {
                    if (pressed.key === "Escape") onCancel();
                    if (pressed.key === "Enter") {
                      pressed.preventDefault();
                      onPost();
                    }
                  }}
                  placeholder="This posts into the conversation."
                  value={draft}
                />
                <button
                  type="button"
                  disabled={draft.trim().length === 0}
                  onClick={onPost}
                  className="border border-border bg-primary px-2 py-0.5 text-[11px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:border-border disabled:bg-transparent disabled:text-faint"
                >
                  Post
                </button>
              </div>
            </div>
          </td>
        </tr>
      ) : null}
    </>
  );
}
