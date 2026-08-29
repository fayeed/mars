import type { BrainSymbol, CommandFailure, IndexSummary } from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { BrainCircuitIcon, DatabaseZapIcon, SearchIcon } from "lucide-react";
import { type FormEvent, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { EntityInspector } from "@/features/brain/EntityInspector";

interface BrainSearchProps {
  onOpenSource: (path: string, line: number) => void;
  repositoryId: string;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

export function BrainSearch({ onOpenSource, repositoryId }: BrainSearchProps) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<BrainSymbol[]>([]);
  const [selected, setSelected] = useState<BrainSymbol | null>(null);
  const [indexSummary, setIndexSummary] = useState<IndexSummary | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function indexProject() {
    setBusy(true);
    setError(null);
    try {
      setIndexSummary(await invoke<IndexSummary>("index_repository", { repositoryId }));
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function search(event: FormEvent) {
    event.preventDefault();
    if (!query.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const matches = await invoke<BrainSymbol[]>("find_symbols", {
        repositoryId,
        query,
        limit: 50,
      });
      setResults(matches);
      setSelected(matches[0] ?? null);
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  return (
    <aside className="flex h-full min-h-0 flex-col bg-card" aria-label="Project Brain">
      <header className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
        <BrainCircuitIcon className="size-4 text-primary" />
        <div className="min-w-0 flex-1">
          <p className="text-xs font-medium">Project Brain</p>
          <p className="text-[10px] uppercase tracking-wider text-muted-foreground">
            {indexSummary
              ? `${indexSummary.indexedFiles} indexed · ${indexSummary.skippedFiles} unchanged`
              : "Structural graph"}
          </p>
        </div>
        <Button disabled={busy} onClick={() => void indexProject()} size="icon-sm" type="button" variant="ghost" aria-label="Index project">
          <DatabaseZapIcon data-icon="inline-start" />
        </Button>
      </header>

      <form className="flex gap-2 border-b p-2" onSubmit={(event) => void search(event)}>
        <Input
          aria-label="Find symbol"
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Find a symbol…"
          value={query}
        />
        <Button aria-label="Search symbols" disabled={busy || !query.trim()} size="icon" type="submit">
          <SearchIcon data-icon="inline-start" />
        </Button>
      </form>

      {error ? <p className="border-b px-3 py-2 text-xs text-destructive">{error}</p> : null}

      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-1 p-2">
          {results.map((symbol) => (
            <Button
              className="h-auto justify-start px-2 py-2 text-left"
              key={symbol.id}
              onClick={() => setSelected(symbol)}
              type="button"
              variant={selected?.id === symbol.id ? "secondary" : "ghost"}
            >
              <span className="min-w-0 flex-1">
                <span className="block truncate text-xs">{symbol.name}</span>
                <span className="block truncate font-mono text-[10px] text-muted-foreground">
                  {symbol.relativePath}:{symbol.startLine}
                </span>
              </span>
              <Badge variant="outline">{symbol.kind}</Badge>
            </Button>
          ))}
          {!busy && query && results.length === 0 ? (
            <p className="px-2 py-6 text-center text-xs text-muted-foreground">No indexed symbols match.</p>
          ) : null}
        </div>
      </ScrollArea>

      {selected ? (
        <EntityInspector
          key={selected.id}
          onOpenSource={onOpenSource}
          repositoryId={repositoryId}
          symbol={selected}
        />
      ) : null}
    </aside>
  );
}
