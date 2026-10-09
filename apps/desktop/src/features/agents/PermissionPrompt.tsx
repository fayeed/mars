import type { AdapterKind, AdapterProbe, AgentAccount, AgentPermissions } from "@mars/shared-types";
import { LogInIcon, RefreshCwIcon, ShieldCheckIcon } from "lucide-react";
import { useState } from "react";

import { billingPhrase, hasOverride } from "@/features/agents/billing";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
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
  /** Who the run will bill. `null` when the CLI could not be asked. */
  account?: AgentAccount | null;
  onSignIn?: () => void;
  onRecheckAccount?: () => void;
  onUseSignIn?: (useSignIn: boolean) => void;
}

export function PermissionPrompt({
  account = null,
  adapter,
  onApprove,
  onOpenChange,
  onRecheckAccount,
  onSignIn,
  onUseSignIn,
  open,
  probe,
}: PermissionPromptProps) {
  const [writeFiles, setWriteFiles] = useState(true);
  const [runCommands, setRunCommands] = useState(true);
  const [network, setNetwork] = useState(false);
  const signedOut = account?.billing.type === "signedOut";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Approve agent capabilities</DialogTitle>
          <DialogDescription>
            {probe?.version ?? adapter} will run only inside this task worktree. These grants last for the current Mars session.
          </DialogDescription>
        </DialogHeader>
        {account ? (
          <AccountNotice
            account={account}
            onRecheck={onRecheckAccount}
            onSignIn={onSignIn}
            onUseSignIn={onUseSignIn}
          />
        ) : null}
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
            disabled={signedOut || (adapter === "codex" && !runCommands)}
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

/**
 * Who pays for the run, said before it starts rather than discovered on a bill.
 *
 * A key variable in Mars's environment wins over a plan sign-in in the
 * non-interactive runs Mars starts, without the one-time question Claude Code
 * asks in a terminal. So the choice is put here, where the run is approved.
 */
function AccountNotice({
  account,
  onRecheck,
  onSignIn,
  onUseSignIn,
}: {
  account: AgentAccount;
  onRecheck?: () => void;
  onSignIn?: () => void;
  onUseSignIn?: (useSignIn: boolean) => void;
}) {
  if (account.billing.type === "signedOut") {
    return (
      <Alert>
        <LogInIcon />
        <AlertTitle>Sign in to the agent first</AlertTitle>
        <AlertDescription>
          <p>
            Mars runs it with your own account and never sees how you sign in.
            {account.signInCommand ? (
              <>
                {" "}Sign in once with <code>{account.signInCommand}</code>, then check again.
              </>
            ) : null}
          </p>
          <div className="mt-2 flex gap-2">
            {onSignIn && account.signInCommand ? (
              <Button onClick={onSignIn} size="sm" type="button">
                <LogInIcon data-icon="inline-start" /> Sign in
              </Button>
            ) : null}
            {onRecheck ? (
              <Button onClick={onRecheck} size="sm" type="button" variant="outline">
                <RefreshCwIcon data-icon="inline-start" /> Check again
              </Button>
            ) : null}
          </div>
        </AlertDescription>
      </Alert>
    );
  }

  const override = account.overrides[0];
  return (
    <div className="grid gap-2 rounded-lg border p-3">
      <p className="text-xs">
        This run bills <strong>{billingPhrase(account.kind, account.billing)}</strong>.
      </p>
      {hasOverride(account) && override ? (
        <div className="flex items-start gap-3">
          <Checkbox
            checked={account.useSignIn}
            disabled={!onUseSignIn}
            id="billing-use-sign-in"
            onCheckedChange={(value) => onUseSignIn?.(Boolean(value))}
          />
          <div className="grid gap-1">
            <Label htmlFor="billing-use-sign-in">
              Use {billingPhrase(account.kind, account.signIn)} instead
            </Label>
            <p className="text-xs text-muted-foreground">
              {override} is set where Mars was started, and the agent uses it over your sign-in
              when Mars runs it. Checked, Mars leaves it out of this agent's environment. Remembered
              for later runs.
            </p>
          </div>
        </div>
      ) : null}
    </div>
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
