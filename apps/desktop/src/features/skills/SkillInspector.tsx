import type { InspectedSkill, SkillInstallation, SkillScope } from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { ShieldCheckIcon } from "lucide-react";
import { useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

interface SkillInspectorProps {
  inspected: InspectedSkill;
  onInstalled: (installation: SkillInstallation) => void;
  repositoryId: string;
  taskId: string;
}

export function SkillInspector({ inspected, onInstalled, repositoryId, taskId }: SkillInspectorProps) {
  const [scope, setScope] = useState<SkillScope>("task");
  const [allowClaude, setAllowClaude] = useState(true);
  const [allowCodex, setAllowCodex] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function install() {
    setBusy(true);
    setError(null);
    try {
      onInstalled(
        await invoke<SkillInstallation>("install_skill", {
          skillId: inspected.id,
          repositoryId,
          taskId,
          scope,
          allowClaude,
          allowCodex,
        }),
      );
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="grid min-h-0 gap-3" aria-label="Skill inspection">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <Badge variant="outline">{inspected.source}</Badge>
        <code className="text-[10px] text-muted-foreground">{inspected.contentHash.slice(0, 16)}</code>
      </div>
      {inspected.changedFiles.length ? (
        <div className="rounded-md border p-2 text-xs">
          <p className="font-medium">Files requiring review</p>
          <ul className="mt-1 grid gap-1 font-mono text-[10px] text-muted-foreground">
            {inspected.changedFiles.map((path) => <li key={path}>{path}</li>)}
          </ul>
        </div>
      ) : null}
      <pre className="max-h-72 overflow-auto whitespace-pre-wrap rounded-md border bg-muted/30 p-3 text-xs leading-relaxed">{inspected.skillMarkdown}</pre>
      <div className="grid gap-2 rounded-md border p-3">
        <Label htmlFor="skill-scope">Activation scope</Label>
        <Select onValueChange={(value) => setScope(value as SkillScope)} value={scope}>
          <SelectTrigger className="w-full" id="skill-scope"><SelectValue /></SelectTrigger>
          <SelectContent>
            <SelectItem value="task">Current task</SelectItem>
            <SelectItem value="workflow">Current workflow</SelectItem>
            <SelectItem value="project">Project</SelectItem>
            <SelectItem value="global">Global</SelectItem>
          </SelectContent>
        </Select>
        <Policy checked={allowClaude} id="skill-claude" label="Allow Claude Code" onChange={setAllowClaude} />
        <Policy checked={allowCodex} id="skill-codex" label="Allow Codex" onChange={setAllowCodex} />
      </div>
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
      <Button disabled={busy || (!allowClaude && !allowCodex)} onClick={() => void install()} type="button">
        <ShieldCheckIcon /> {busy ? "Installing…" : "Trust, pin, and install"}
      </Button>
    </section>
  );
}

function Policy({ checked, id, label, onChange }: { checked: boolean; id: string; label: string; onChange: (checked: boolean) => void }) {
  return (
    <div className="flex items-center gap-2">
      <Checkbox checked={checked} id={id} onCheckedChange={(value) => onChange(Boolean(value))} />
      <Label htmlFor={id}>{label}</Label>
    </div>
  );
}
