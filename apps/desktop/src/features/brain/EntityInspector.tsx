import type { BrainNeighborhood, BrainSymbol, CommandFailure } from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { ArrowUpRightIcon, GitForkIcon } from "lucide-react";
import { useEffect, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

interface EntityInspectorProps {
  onOpenSource: (path: string, line: number) => void;
  repositoryId: string;
  symbol: BrainSymbol;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

export function EntityInspector({ onOpenSource, repositoryId, symbol }: EntityInspectorProps) {
  const [neighborhood, setNeighborhood] = useState<BrainNeighborhood | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void invoke<BrainNeighborhood>("brain_related_code", {
      repositoryId,
      symbolId: symbol.id,
      depth: 2,
      limit: 25,
    })
      .then((result) => {
        if (active) setNeighborhood(result);
      })
      .catch((caught: unknown) => {
        if (active) setError(messageFor(caught));
      });
    return () => {
      active = false;
    };
  }, [repositoryId, symbol.id]);

  return (
    <section className="flex flex-col gap-3 border-t p-3" aria-label={`${symbol.name} graph`}>
      <div className="flex items-start gap-2">
        <GitForkIcon className="mt-0.5 size-4 text-primary" />
        <div className="min-w-0 flex-1">
          <p className="truncate text-xs font-medium">{symbol.qualifiedName}</p>
          <p className="truncate text-[10px] text-muted-foreground">
            {symbol.relativePath}:{symbol.startLine}
          </p>
        </div>
        <Badge variant="outline">{symbol.kind}</Badge>
      </div>

      <Button
        className="justify-start"
        onClick={() => onOpenSource(symbol.relativePath, symbol.startLine)}
        size="sm"
        type="button"
        variant="secondary"
      >
        <ArrowUpRightIcon data-icon="inline-start" /> Open definition
      </Button>

      {error ? <p className="text-xs text-destructive">{error}</p> : null}
      {!neighborhood && !error ? <p className="text-xs text-muted-foreground">Tracing relationships…</p> : null}

      {neighborhood?.edges.map((edge) => (
        <button
          className="rounded-md border p-2 text-left transition-colors hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          key={edge.id}
          onClick={() => onOpenSource(edge.evidence.relativePath, edge.evidence.startLine)}
          type="button"
        >
          <span className="flex items-center justify-between gap-2 text-[10px] uppercase tracking-wider text-muted-foreground">
            {edge.edgeKind}
            <span>{Math.round(edge.confidence * 100)}%</span>
          </span>
          <span className="mt-1 block truncate text-xs">
            {edge.source?.name ?? edge.evidence.relativePath} → {edge.target?.name ?? edge.targetKey}
          </span>
          <span className="mt-1 block truncate font-mono text-[10px] text-muted-foreground">
            {edge.evidence.relativePath}:{edge.evidence.startLine}
          </span>
        </button>
      ))}
    </section>
  );
}
