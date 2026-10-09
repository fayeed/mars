import type {
  BrainAnswer as BrainAnswerValue,
  BrainNeighborhood,
  BrainSymbol,
  CommandFailure,
  DependencyNode,
  DependencySlice,
  IndexStatus,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { ArrowLeftIcon, CrosshairIcon, FileCode2Icon, SearchIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { cn } from "@/lib/utils";
import { BrainAnswerMessage } from "@/features/brain/BrainAnswerMessage";
import { WhyAnswer } from "@/features/brain/WhyAnswer";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateBlock, StateLabel } from "@/shell/primitives";

export interface CodeGraphProps {
  repositoryId: string;
  index: IndexStatus | null;
  /** What is being indexed right now, while it is. */
  indexProgress?: string | null;
  /** Opens the graph on this file or symbol id. */
  initialTarget?: string | null;
  /** A file worth offering as a starting point — the one open in the editor. */
  suggestion?: string | null;
  onOpen: (path: string, line: number | null) => void;
  onReindex?: () => void;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

// Layout, in SVG units.
const NODE_WIDTH = 196;
const NODE_HEIGHT = 22;
const ROW = 30;
const COLUMN = 260;
const PAD = 16;

interface Placed {
  node: DependencyNode;
  column: number;
  x: number;
  y: number;
}

/**
 * A slice laid out in columns: the target in the middle, what depends on it
 * to the left and what it depends on to the right, one column further out
 * per step away. Columns are ordered by name so the same slice always draws
 * the same way.
 */
export function layoutSlice(slice: DependencySlice): {
  placed: Map<string, Placed>;
  width: number;
  height: number;
  center: string | null;
} {
  const center =
    slice.nodes.find((node) => node.id === slice.target)?.id ??
    slice.nodes.find((node) => node.distance === 0)?.id ??
    slice.nodes[0]?.id ??
    null;
  const incoming = new Set<string>();
  for (const edge of slice.edges) {
    if (edge.incoming) incoming.add(edge.source);
  }
  const columns = new Map<number, DependencyNode[]>();
  for (const node of slice.nodes) {
    const column =
      node.id === center ? 0 : (incoming.has(node.id) ? -1 : 1) * Math.max(1, node.distance);
    columns.set(column, [...(columns.get(column) ?? []), node]);
  }
  const keys = [...columns.keys()];
  const minColumn = Math.min(0, ...keys);
  const maxColumn = Math.max(0, ...keys);
  const tallest = Math.max(1, ...[...columns.values()].map((nodes) => nodes.length));
  const height = PAD * 2 + tallest * ROW;
  const placed = new Map<string, Placed>();
  for (const [column, nodes] of columns) {
    const sorted = [...nodes].sort((left, right) => left.label.localeCompare(right.label));
    const top = (height - sorted.length * ROW) / 2;
    sorted.forEach((node, row) => {
      placed.set(node.id, {
        node,
        column,
        x: PAD + (column - minColumn) * COLUMN,
        y: top + row * ROW + (ROW - NODE_HEIGHT) / 2,
      });
    });
  }
  return {
    placed,
    width: PAD * 2 + (maxColumn - minColumn) * COLUMN + NODE_WIDTH,
    height,
    center,
  };
}

function truncate(text: string, length: number) {
  return text.length > length ? `…${text.slice(text.length - length + 1)}` : text;
}

/** Whether a node is a symbol, which can be explained and has a blast radius. */
function isSymbol(slice: DependencySlice, node: DependencyNode) {
  return slice.targetKind === "symbol" && node.kind !== "file" && node.resolved;
}

/** What to ask the graph for to put this node in the middle. */
function exploreTarget(slice: DependencySlice, node: DependencyNode) {
  return slice.targetKind === "symbol" ? node.id : node.path;
}

type Answer = { title: string; state: ActivityState; answer: BrainAnswerValue | null; error: string | null };

/**
 * How the code fits together, drawn from the local index.
 *
 * Pick a file or a symbol and the graph shows what uses it and what it uses,
 * with how sure the index is of each edge and what it could not resolve.
 * Any node can be put in the middle, opened, explained, or asked how far a
 * change to it would reach. It is the same graph agents query over MCP, so
 * what is here is what they see.
 */
export function CodeGraph({
  index,
  indexProgress,
  initialTarget,
  onOpen,
  onReindex,
  repositoryId,
  suggestion,
}: CodeGraphProps) {
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<BrainSymbol[]>([]);
  const [trail, setTrail] = useState<{ target: string; label: string }[]>(
    initialTarget ? [{ target: initialTarget, label: initialTarget }] : [],
  );
  // Kept with the target it was read for: the core normalises a target (a
  // path's separators, say), so the slice's own name for it can differ.
  const [loaded, setLoaded] = useState<{
    target: string;
    slice: DependencySlice | null;
    error: string | null;
  } | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [answer, setAnswer] = useState<Answer | null>(null);
  const [reach, setReach] = useState<BrainNeighborhood | null>(null);
  const [concept, setConcept] = useState("");

  const current = trail.at(-1) ?? null;
  const settled = current !== null && loaded?.target === current.target ? loaded : null;
  const slice = settled?.slice ?? null;
  const sliceError = settled?.error ?? null;
  const loadingSlice = current !== null && settled === null;

  // Symbols matching what is typed, as it is typed.
  useEffect(() => {
    const text = query.trim();
    if (text.length < 2) return;
    let active = true;
    const timer = setTimeout(() => {
      invoke<BrainSymbol[]>("find_symbols", { repositoryId, query: text, limit: 12 })
        .then((symbols) => {
          if (active) setFound(symbols);
        })
        .catch(() => {
          if (active) setFound([]);
        });
    }, 150);
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [query, repositoryId]);

  // The slice for whatever is in the middle.
  const currentTarget = current?.target ?? null;
  useEffect(() => {
    if (currentTarget === null) return;
    let active = true;
    invoke<DependencySlice>("brain_dependency_graph", { repositoryId, target: currentTarget })
      .then((next) => {
        if (!active) return;
        setLoaded({ target: currentTarget, slice: next, error: null });
        // Opened from an id, the step is named by it until the graph says
        // what it is called.
        const named = layoutSlice(next).center;
        const label = next.nodes.find((node) => node.id === named)?.label;
        if (!label) return;
        setTrail((previous) =>
          previous.map((step, position) =>
            position === previous.length - 1 &&
            step.target === currentTarget &&
            step.label === step.target
              ? { ...step, label }
              : step,
          ),
        );
      })
      .catch((caught: unknown) => {
        if (active) setLoaded({ target: currentTarget, slice: null, error: messageFor(caught) });
      });
    return () => {
      active = false;
    };
  }, [currentTarget, repositoryId]);

  const layout = useMemo(() => (slice ? layoutSlice(slice) : null), [slice]);
  const selectedNode =
    (slice && layout && slice.nodes.find((node) => node.id === (selected ?? layout.center))) ?? null;

  function explore(target: string, label: string) {
    setTrail((previous) => [...previous, { target, label }]);
    setSelected(null);
    setAnswer(null);
    setReach(null);
    setQuery("");
    setFound([]);
  }

  function back(to: number) {
    setTrail((previous) => previous.slice(0, to + 1));
    setSelected(null);
    setAnswer(null);
    setReach(null);
  }

  async function ask(title: string, command: string, args: Record<string, unknown>) {
    setAnswer({ title, state: "loading", answer: null, error: null });
    try {
      const result = await invoke<BrainAnswerValue>(command, { repositoryId, ...args });
      setAnswer({ title, state: "done", answer: result, error: null });
    } catch (caught) {
      setAnswer({ title, state: "failed", answer: null, error: messageFor(caught) });
    }
  }

  async function blastRadius(node: DependencyNode) {
    setReach(null);
    try {
      setReach(
        await invoke<BrainNeighborhood>("brain_blast_radius", {
          repositoryId,
          symbolId: node.id,
          depth: 3,
          limit: 60,
        }),
      );
    } catch (caught) {
      setAnswer({ title: "How far a change reaches", state: "failed", answer: null, error: messageFor(caught) });
    }
  }

  const cold = !index || index.counts.files === 0;
  const looksLikePath = /[/.]/.test(query.trim());

  return (
    <div className="flex h-full min-h-0 flex-col" data-slot="code-graph">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-2 py-1.5">
        <form
          className="relative flex min-w-56 flex-1 items-center gap-1"
          onSubmit={(event) => {
            event.preventDefault();
            const text = query.trim();
            if (!text) return;
            const first = found[0];
            if (first && !looksLikePath) explore(first.id, first.qualifiedName || first.name);
            else explore(text, text);
          }}
        >
          <SearchIcon className="size-3.5 shrink-0 text-faint" />
          <input
            aria-label="Find a symbol or file"
            className="min-w-0 flex-1 bg-transparent text-[12px] text-foreground outline-none placeholder:text-faint"
            onChange={(event) => {
              setQuery(event.target.value);
              if (event.target.value.trim().length < 2) setFound([]);
            }}
            placeholder="A symbol, or a file path like src/server.ts"
            value={query}
          />
          {query.trim().length >= 2 && (found.length > 0 || looksLikePath) ? (
            <ul
              aria-label="Matches"
              className="absolute top-full left-0 z-10 mt-1 max-h-72 w-full overflow-auto border border-border bg-surface shadow-sm"
            >
              {looksLikePath ? (
                <li>
                  <button
                    type="button"
                    onClick={() => explore(query.trim(), query.trim())}
                    className="flex w-full items-center gap-2 px-2 py-1 text-left outline-none hover:bg-surface-raised focus-visible:bg-surface-raised"
                  >
                    <FileCode2Icon className="size-3 text-faint" />
                    <Mono className="text-foreground">{query.trim()}</Mono>
                    <Mono className="ml-auto text-faint">file</Mono>
                  </button>
                </li>
              ) : null}
              {found.map((symbol) => (
                <li key={symbol.id}>
                  <button
                    type="button"
                    onClick={() => explore(symbol.id, symbol.qualifiedName || symbol.name)}
                    className="flex w-full items-baseline gap-2 px-2 py-1 text-left outline-none hover:bg-surface-raised focus-visible:bg-surface-raised"
                  >
                    <span className="min-w-0 truncate text-[12px] text-foreground">
                      {symbol.qualifiedName || symbol.name}
                    </span>
                    <Mono className="shrink-0 text-faint">{symbol.kind}</Mono>
                    <Mono className="ml-auto shrink-0 truncate text-faint">
                      {symbol.relativePath}:{symbol.startLine}
                    </Mono>
                  </button>
                </li>
              ))}
            </ul>
          ) : null}
        </form>
        <form
          className="flex min-w-56 flex-1 items-center gap-1"
          onSubmit={(event) => {
            event.preventDefault();
            if (concept.trim()) void ask(`Where is ${concept.trim()}?`, "brain_where_is", { concept: concept.trim() });
          }}
        >
          <input
            aria-label="Where is"
            className="min-w-0 flex-1 border-b border-border bg-transparent text-[12px] text-foreground outline-none placeholder:text-faint"
            onChange={(event) => setConcept(event.target.value)}
            placeholder="Where is… (e.g. rate limiting)"
            value={concept}
          />
          <button
            type="submit"
            disabled={!concept.trim()}
            className="border border-border px-2 py-0.5 text-[11px] outline-none hover:border-border-strong disabled:text-faint"
          >
            Find
          </button>
        </form>
      </div>

      <div className="flex flex-wrap items-center gap-2 border-b border-border px-2 py-1">
        {index ? (
          <StateLabel
            state={indexProgress ? "streaming" : cold ? "idle" : index.headMatchesIndex ? "done" : "stale"}
            detail={
              indexProgress ??
              (cold
                ? "not indexed yet"
                : `${index.counts.files.toLocaleString()} files · ${index.counts.symbols.toLocaleString()} symbols${index.headMatchesIndex ? "" : " · behind the working tree"}`)
            }
          />
        ) : (
          <StateLabel state="loading" detail="reading the index" />
        )}
        {onReindex ? (
          <button
            type="button"
            onClick={onReindex}
            disabled={Boolean(indexProgress)}
            className="border border-border px-1.5 py-0.5 text-[11px] text-muted-foreground outline-none hover:border-border-strong disabled:text-faint"
          >
            {cold ? "Index now" : "Re-index"}
          </button>
        ) : null}
        {trail.length > 0 ? (
          <nav aria-label="Explored" className="ml-auto flex min-w-0 items-center gap-1 overflow-hidden">
            {trail.length > 1 ? (
              <button
                type="button"
                aria-label="Back"
                onClick={() => back(trail.length - 2)}
                className="text-faint outline-none hover:text-foreground"
              >
                <ArrowLeftIcon className="size-3.5" />
              </button>
            ) : null}
            {trail.map((step, position) => (
              <button
                key={`${step.target}:${position}`}
                type="button"
                aria-current={position === trail.length - 1 ? "page" : undefined}
                onClick={() => back(position)}
                className={cn(
                  "min-w-0 truncate outline-none hover:underline",
                  position === trail.length - 1 ? "text-foreground" : "text-faint",
                )}
              >
                <Mono>{truncate(step.label, 32)}</Mono>
              </button>
            ))}
          </nav>
        ) : null}
      </div>

      <div className="flex min-h-0 flex-1">
        <div className="min-w-0 flex-1 overflow-auto">
          {!current ? (
            <StateBlock
              state="idle"
              title={cold ? "The index is empty" : "Pick something to look at"}
              description={
                cold
                  ? "Mars reads this repository into a local index — nothing leaves the machine — and keeps it current as files change."
                  : "Find a symbol or a file above to see what it uses and what uses it."
              }
              action={
                suggestion && !cold ? (
                  <button
                    type="button"
                    onClick={() => explore(suggestion, suggestion)}
                    className="flex items-center gap-1 border border-border px-2 py-0.5 text-[11px] outline-none hover:border-border-strong"
                  >
                    <CrosshairIcon className="size-3" />
                    Start from {suggestion}
                  </button>
                ) : undefined
              }
            />
          ) : sliceError ? (
            <StateBlock state="failed" title={`Nothing to draw for ${current.label}`} description={sliceError} />
          ) : loadingSlice || !slice || !layout ? (
            <StateBlock state="loading" title={`Reading the graph around ${current.label}`} />
          ) : (
            <svg
              role="img"
              aria-label={`Dependencies of ${current.label}`}
              width={layout.width}
              height={layout.height}
              viewBox={`0 0 ${layout.width} ${layout.height}`}
              className="block"
            >
              {slice.edges.map((edge) => {
                const from = layout.placed.get(edge.source);
                const to = layout.placed.get(edge.target);
                if (!from || !to) return null;
                const [left, right] = from.x <= to.x ? [from, to] : [to, from];
                const x1 = left.x + NODE_WIDTH;
                const y1 = left.y + NODE_HEIGHT / 2;
                const x2 = right.x;
                const y2 = right.y + NODE_HEIGHT / 2;
                const bend = Math.max(24, (x2 - x1) / 2);
                return (
                  <path
                    key={`${edge.kind}:${edge.source}:${edge.target}`}
                    d={`M ${x1} ${y1} C ${x1 + bend} ${y1}, ${x2 - bend} ${y2}, ${x2} ${y2}`}
                    fill="none"
                    stroke="var(--border-strong)"
                    strokeDasharray={edge.confidence < 0.6 ? "3 3" : undefined}
                    strokeWidth={1}
                  >
                    <title>
                      {edge.kind} · {Math.round(edge.confidence * 100)}% sure
                    </title>
                  </path>
                );
              })}
              {[...layout.placed.values()].map(({ node, x, y }) => {
                const isCenter = node.id === layout.center;
                const isSelected = node.id === (selected ?? layout.center);
                return (
                  <g
                    key={node.id}
                    role="button"
                    tabIndex={0}
                    aria-label={`${node.label} (${node.kind})`}
                    aria-pressed={isSelected}
                    transform={`translate(${x} ${y})`}
                    className="cursor-pointer outline-none [&:focus-visible>rect]:stroke-[var(--ring)]"
                    onClick={() => setSelected(node.id)}
                    onDoubleClick={() => {
                      if (!isCenter && node.resolved) explore(exploreTarget(slice, node), node.label);
                    }}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        setSelected(node.id);
                      }
                    }}
                  >
                    <rect
                      width={NODE_WIDTH}
                      height={NODE_HEIGHT}
                      rx={2}
                      fill={isCenter ? "var(--primary)" : "var(--surface)"}
                      stroke={isSelected ? "var(--primary)" : "var(--border)"}
                      strokeDasharray={node.resolved ? undefined : "3 2"}
                    />
                    <text
                      x={8}
                      y={NODE_HEIGHT / 2}
                      dominantBaseline="central"
                      fontSize={11}
                      fontFamily="var(--font-mono, monospace)"
                      fill={isCenter ? "var(--primary-foreground)" : node.resolved ? "var(--foreground)" : "var(--muted-foreground)"}
                    >
                      {truncate(node.label, 28)}
                    </text>
                  </g>
                );
              })}
            </svg>
          )}
          {slice && !loadingSlice && (slice.truncated || slice.gaps.length > 0) ? (
            <div className="flex flex-col gap-0.5 border-t border-border px-3 py-2">
              {slice.truncated ? (
                <Mono className="text-primary">Cut short: there was more than fits on one screen.</Mono>
              ) : null}
              {slice.gaps.length > 0 ? <MetaLabel>What the graph could not resolve</MetaLabel> : null}
              {slice.gaps.map((gap) => (
                <span className="text-[11px] text-muted-foreground" key={`${gap.kind}:${gap.subject}`}>
                  <Mono className="text-faint">{gap.kind}</Mono> {gap.subject} — {gap.detail}
                </span>
              ))}
            </div>
          ) : null}
        </div>

        <aside
          aria-label="Selected"
          className="flex w-80 shrink-0 flex-col gap-2 overflow-auto border-l border-border p-2"
        >
          {selectedNode && slice ? (
            <>
              <div className="flex flex-col gap-0.5">
                <MetaLabel>{selectedNode.kind}</MetaLabel>
                <span className="break-all text-[13px] text-foreground">{selectedNode.label}</span>
                {selectedNode.resolved ? (
                  <Mono className="break-all text-faint">
                    {selectedNode.path}
                    {selectedNode.lineStart > 0 ? `:${selectedNode.lineStart}` : ""}
                  </Mono>
                ) : (
                  <Mono className="text-faint">outside the repository</Mono>
                )}
              </div>
              <div className="flex flex-wrap gap-1">
                {selectedNode.resolved ? (
                  <button
                    type="button"
                    onClick={() => onOpen(selectedNode.path, selectedNode.lineStart || null)}
                    className="border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong"
                  >
                    Open
                  </button>
                ) : null}
                {selectedNode.id !== layout?.center && selectedNode.resolved ? (
                  <button
                    type="button"
                    onClick={() => explore(exploreTarget(slice, selectedNode), selectedNode.label)}
                    className="border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong"
                  >
                    Put in the middle
                  </button>
                ) : null}
                {isSymbol(slice, selectedNode) ? (
                  <>
                    <button
                      type="button"
                      onClick={() =>
                        void ask(`What does ${selectedNode.label} do?`, "brain_explain", {
                          symbolId: selectedNode.id,
                        })
                      }
                      className="border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong"
                    >
                      Explain
                    </button>
                    <button
                      type="button"
                      onClick={() => void blastRadius(selectedNode)}
                      className="border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong"
                    >
                      What a change reaches
                    </button>
                  </>
                ) : null}
              </div>
              <EdgeList slice={slice} node={selectedNode} onSelect={setSelected} />
              {reach ? <Reach reach={reach} onOpen={onOpen} /> : null}
              {isSymbol(slice, selectedNode) ? (
                // The decisions and history behind it, from approved knowledge.
                <WhyAnswer
                  key={selectedNode.id}
                  onOpenCode={(path, line) => onOpen(path, line)}
                  repositoryId={repositoryId}
                  subject={{ kind: "symbol", id: selectedNode.id }}
                />
              ) : null}
            </>
          ) : (
            <p className="text-[11px] text-muted-foreground">
              Click a box to see it here; double-click to put it in the middle.
            </p>
          )}
          {answer ? (
            <BrainAnswerMessage
              answer={answer.answer}
              error={answer.error}
              onOpenCitation={(path, line) => onOpen(path, line)}
              question={answer.title}
              state={answer.state}
            />
          ) : null}
        </aside>
      </div>
    </div>
  );
}

function EdgeList({
  node,
  onSelect,
  slice,
}: {
  node: DependencyNode;
  onSelect: (id: string) => void;
  slice: DependencySlice;
}) {
  const uses = slice.edges.filter((edge) => edge.source === node.id);
  const usedBy = slice.edges.filter((edge) => edge.target === node.id);
  const label = (id: string) => slice.nodes.find((candidate) => candidate.id === id)?.label ?? id;
  const group = (title: string, edges: typeof uses, other: (edge: (typeof uses)[number]) => string) =>
    edges.length > 0 ? (
      <section aria-label={title} className="flex flex-col gap-0.5">
        <div className="flex items-baseline gap-2">
          <MetaLabel>{title}</MetaLabel>
          <Mono className="text-faint">{edges.length}</Mono>
        </div>
        <ul className="flex flex-col">
          {edges.map((edge) => (
            <li key={`${edge.kind}:${edge.source}:${edge.target}`}>
              <button
                type="button"
                onClick={() => onSelect(other(edge))}
                className="flex w-full items-baseline gap-2 text-left outline-none hover:underline"
              >
                <Mono className="w-16 shrink-0 text-faint">{edge.kind}</Mono>
                <Mono className="min-w-0 flex-1 truncate text-foreground">{label(other(edge))}</Mono>
                <Mono className="shrink-0 text-faint">{Math.round(edge.confidence * 100)}%</Mono>
              </button>
            </li>
          ))}
        </ul>
      </section>
    ) : null;
  return (
    <>
      {group("Uses", uses, (edge) => edge.target)}
      {group("Used by", usedBy, (edge) => edge.source)}
    </>
  );
}

function Reach({
  onOpen,
  reach,
}: {
  onOpen: (path: string, line: number | null) => void;
  reach: BrainNeighborhood;
}) {
  const affected = reach.symbols.filter((symbol) => symbol.id !== reach.root.id);
  const files = new Set(affected.map((symbol) => symbol.relativePath));
  return (
    <section aria-label="What a change reaches" className="flex flex-col gap-0.5">
      <MetaLabel>What a change reaches</MetaLabel>
      <p className="text-[11px] text-muted-foreground">
        {affected.length === 0
          ? "Nothing in the index depends on it."
          : `${affected.length} symbol(s) in ${files.size} file(s) depend on ${reach.root.name}, directly or through others.`}
      </p>
      <ul className="flex flex-col">
        {affected.slice(0, 30).map((symbol) => (
          <li key={symbol.id}>
            <button
              type="button"
              onClick={() => onOpen(symbol.relativePath, symbol.startLine)}
              className="flex w-full items-baseline gap-2 text-left outline-none hover:underline"
            >
              <Mono className="min-w-0 flex-1 truncate text-foreground">{symbol.qualifiedName || symbol.name}</Mono>
              <Mono className="shrink-0 truncate text-faint">{symbol.relativePath}</Mono>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
