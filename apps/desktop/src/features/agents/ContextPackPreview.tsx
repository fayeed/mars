import type { ContextPack } from "@mars/shared-types";
import { CheckIcon, MinusIcon, PlusIcon, ShieldAlertIcon } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";

interface ContextPackPreviewProps {
  busy: boolean;
  onContinue: () => void;
  onOpenChange: (open: boolean) => void;
  onToggle: (entityRef: string, included: boolean) => void;
  open: boolean;
  pack: ContextPack | null;
}

export function ContextPackPreview({
  busy,
  onContinue,
  onOpenChange,
  onToggle,
  open,
  pack,
}: ContextPackPreviewProps) {
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-h-[88vh] sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>Review context sent to the agent</DialogTitle>
          <DialogDescription>
            This is the exact explainable pack Mars will prepend to your assignment. Remove anything the agent does not need.
          </DialogDescription>
        </DialogHeader>
        {pack ? (
          <>
            <div className="flex flex-wrap items-center gap-2 text-xs">
              <Badge variant="outline">{formatBytes(pack.totalBytes)} / {formatBytes(pack.maxBytes)}</Badge>
              {pack.sectionUsage.map((usage) => (
                <span className="text-muted-foreground" key={usage.section}>
                  {usage.section} {formatBytes(usage.usedBytes)}
                </span>
              ))}
            </div>
            <ScrollArea className="max-h-[55vh] pr-3">
              <div className="grid gap-3">
                {pack.items.map((item) => (
                  <article className="grid gap-2 rounded-lg border bg-muted/15 p-3" key={item.entityRef}>
                    <div className="flex items-start justify-between gap-3">
                      <div className="grid gap-1">
                        <div className="flex flex-wrap items-center gap-2">
                          <Badge>{item.section}</Badge>
                          <h3 className="text-sm font-medium">{item.title}</h3>
                        </div>
                        <p className="text-xs text-muted-foreground">{item.reason}</p>
                      </div>
                      <Button
                        aria-label={`Remove ${item.title}`}
                        onClick={() => onToggle(item.entityRef, false)}
                        size="icon-sm"
                        type="button"
                        variant="ghost"
                      >
                        <MinusIcon />
                      </Button>
                    </div>
                    <pre className="max-h-28 overflow-auto whitespace-pre-wrap rounded-md bg-background p-2 text-[11px] leading-relaxed">{item.content}</pre>
                    <div className="flex flex-wrap gap-x-3 gap-y-1 text-[10px] text-muted-foreground">
                      <span>{item.provenance}</span>
                      <span>{item.freshness}</span>
                      <span>{item.sourceLocation}</span>
                    </div>
                  </article>
                ))}
                {pack.exclusions.length ? (
                  <section className="grid gap-2 rounded-lg border border-dashed p-3" aria-label="Excluded context">
                    <div className="flex items-center gap-2 text-xs font-medium">
                      <ShieldAlertIcon className="size-3.5" /> Excluded
                    </div>
                    {pack.exclusions.map((item) => {
                      const userRemoved = item.reason === "removed in pack preview";
                      return (
                        <div className="flex items-center justify-between gap-3 text-xs" key={`${item.entityRef}:${item.reason}`}>
                          <span className="min-w-0 truncate text-muted-foreground">{item.entityRef} · {item.reason}</span>
                          {userRemoved ? (
                            <Button onClick={() => onToggle(item.entityRef, true)} size="xs" type="button" variant="outline">
                              <PlusIcon /> Add
                            </Button>
                          ) : null}
                        </div>
                      );
                    })}
                  </section>
                ) : null}
              </div>
            </ScrollArea>
          </>
        ) : null}
        <DialogFooter>
          <Button disabled={busy || !pack} onClick={onContinue} type="button">
            <CheckIcon /> Continue to permissions
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function formatBytes(bytes: number) {
  return bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} KiB`;
}
