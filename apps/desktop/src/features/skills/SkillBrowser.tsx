import type {
  InspectedSkill,
  SkillCandidate,
  SkillInstallation,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { PackageSearchIcon, Trash2Icon } from "lucide-react";
import { type FormEvent, useEffect, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { SkillInspector } from "@/features/skills/SkillInspector";

interface SkillBrowserProps {
  repositoryId: string;
  taskId: string;
}

export function SkillBrowser({ repositoryId, taskId }: SkillBrowserProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [candidates, setCandidates] = useState<SkillCandidate[]>([]);
  const [inspected, setInspected] = useState<InspectedSkill | null>(null);
  const [installed, setInstalled] = useState<SkillInstallation[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    invoke<SkillInstallation[]>("list_task_skills", { taskId })
      .then((items) => {
        if (active) setInstalled(items);
      })
      .catch((caught: unknown) => {
        if (active) setError(messageFor(caught));
      });
    return () => { active = false; };
  }, [taskId]);

  async function search(event: FormEvent) {
    event.preventDefault();
    if (query.trim().length < 2) return;
    setBusy(true);
    setError(null);
    try {
      setCandidates(await invoke<SkillCandidate[]>("search_skills", { query: query.trim() }));
      setInspected(null);
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function inspect(candidate: SkillCandidate) {
    setBusy(true);
    setError(null);
    try {
      setInspected(await invoke<InspectedSkill>("inspect_skill", { candidate }));
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function remove(installationId: string) {
    setError(null);
    try {
      await invoke("remove_skill_installation", { installationId });
      setInstalled((current) => current.filter((item) => item.id !== installationId));
    } catch (caught) {
      setError(messageFor(caught));
    }
  }

  return (
    <section className="grid gap-2 rounded-lg border bg-muted/10 p-3" aria-label="Task skills">
      <header className="flex items-center justify-between gap-2">
        <div>
          <h3 className="text-xs font-medium">Task skills</h3>
          <p className="text-[10px] text-muted-foreground">Inspected instructions available to approved agents.</p>
        </div>
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogTrigger render={<Button size="sm" type="button" variant="outline" />}>
            <PackageSearchIcon /> Find skills
          </DialogTrigger>
          <DialogContent className="max-h-[86vh] overflow-auto sm:max-w-2xl">
            <DialogHeader>
              <DialogTitle>{inspected ? `Inspect ${inspected.name}` : "Discover skills.sh"}</DialogTitle>
              <DialogDescription>Discovery never activates a skill. Review SKILL.md and every changed file before trust.</DialogDescription>
            </DialogHeader>
            {inspected ? (
              <SkillInspector
                inspected={inspected}
                onInstalled={(installation) => {
                  setInstalled((current) => [installation, ...current.filter((item) => item.id !== installation.id)]);
                  setOpen(false);
                }}
                repositoryId={repositoryId}
                taskId={taskId}
              />
            ) : (
              <>
                <form className="flex gap-2" onSubmit={(event) => void search(event)}>
                  <Input aria-label="Search skills.sh" onChange={(event) => setQuery(event.target.value)} placeholder="React testing, accessibility…" value={query} />
                  <Button disabled={busy || query.trim().length < 2} type="submit">Search</Button>
                </form>
                <div className="grid gap-2">
                  {candidates.map((candidate) => (
                    <button className="flex items-center gap-3 rounded-md border p-3 text-left hover:bg-muted" key={`${candidate.source}/${candidate.name}`} onClick={() => void inspect(candidate)} type="button">
                      <span className="min-w-0 flex-1"><strong className="block truncate text-xs">{candidate.name}</strong><span className="block truncate text-[10px] text-muted-foreground">{candidate.source}</span></span>
                      <Badge variant="outline">{candidate.installs.toLocaleString()}</Badge>
                    </button>
                  ))}
                </div>
              </>
            )}
            {error ? <p className="text-xs text-destructive" role="alert">{error}</p> : null}
          </DialogContent>
        </Dialog>
      </header>
      {installed.map((installation) => (
        <article className="flex items-center gap-2 rounded-md border bg-card p-2" key={installation.id}>
          <span className="min-w-0 flex-1"><strong className="block truncate text-xs">{installation.name}</strong><span className="font-mono text-[10px] text-muted-foreground">{installation.pinnedHash.slice(0, 12)} · {installation.scope}</span></span>
          <div className="flex gap-1">{installation.allowClaude ? <Badge variant="outline">Claude</Badge> : null}{installation.allowCodex ? <Badge variant="outline">Codex</Badge> : null}</div>
          <Button aria-label={`Remove ${installation.name}`} onClick={() => void remove(installation.id)} size="icon-sm" type="button" variant="ghost"><Trash2Icon /></Button>
        </article>
      ))}
      {installed.length === 0 ? <p className="py-2 text-center text-xs text-muted-foreground">No skills attached.</p> : null}
    </section>
  );
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) return String((error as { message: unknown }).message);
  return error instanceof Error ? error.message : String(error);
}
