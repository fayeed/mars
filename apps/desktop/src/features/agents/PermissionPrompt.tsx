import type { AdapterKind, AdapterProbe, AgentPermissions } from "@mars/shared-types";
import { ShieldCheckIcon } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";

interface PermissionPromptProps {
  adapter: AdapterKind;
  onApprove: (permissions: AgentPermissions) => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  probe: AdapterProbe | null;
}

export function PermissionPrompt({ adapter, onApprove, onOpenChange, open, probe }: PermissionPromptProps) {
  const [writeFiles, setWriteFiles] = useState(true);
  const [runCommands, setRunCommands] = useState(true);
  const [network, setNetwork] = useState(false);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Approve agent capabilities</DialogTitle>
          <DialogDescription>
            {probe?.version ?? adapter} will run only inside this task worktree. These grants last for the current Mars session.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <PermissionRow checked={writeFiles} description="Create and edit files inside the isolated worktree." id="permission-write" label="Write worktree files" onChange={setWriteFiles} />
          <PermissionRow checked={runCommands} description="Run local commands and tests inside the worktree." id="permission-command" label="Run commands" onChange={setRunCommands} />
          <PermissionRow checked={network} description="Allow provider tools that can access remote services." id="permission-network" label="Use network tools" onChange={setNetwork} />
        </div>
        {adapter === "codex" && !runCommands ? (
          <p className="text-xs text-destructive">Codex CLI requires command permission for this integration.</p>
        ) : null}
        <DialogFooter>
          <Button
            disabled={adapter === "codex" && !runCommands}
            onClick={() => onApprove({ writeFiles, runCommands, network })}
            type="button"
          >
            <ShieldCheckIcon data-icon="inline-start" /> Approve and launch
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function PermissionRow({ checked, description, id, label, onChange }: { checked: boolean; description: string; id: string; label: string; onChange: (checked: boolean) => void }) {
  return (
    <div className="flex items-start gap-3 rounded-lg border p-3">
      <Checkbox checked={checked} id={id} onCheckedChange={(value) => onChange(Boolean(value))} />
      <div className="grid gap-1">
        <Label htmlFor={id}>{label}</Label>
        <p className="text-xs text-muted-foreground">{description}</p>
      </div>
    </div>
  );
}
