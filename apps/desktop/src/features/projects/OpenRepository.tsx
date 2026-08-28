import type {
  CommandFailure,
  OpenRepositoryOutcome,
  ProjectSnapshot,
  RepositoryCandidate,
  RepositoryCapability,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { AlertCircleIcon, FolderOpenIcon, ShieldCheckIcon } from "lucide-react";
import { useState } from "react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

const capabilityLabels: Record<RepositoryCapability, string> = {
  readFiles: "Read source files",
  indexCode: "Build local code index",
  observeGit: "Read Git metadata",
};

interface OpenRepositoryProps {
  onOpened: (project: ProjectSnapshot) => void;
}

function RepositoryError({ message }: { message: string }) {
  return (
    <Alert variant="destructive">
      <AlertCircleIcon />
      <AlertTitle>Repository could not be opened</AlertTitle>
      <AlertDescription>{message}</AlertDescription>
    </Alert>
  );
}

function errorMessage(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

export function OpenRepository({ onOpened }: OpenRepositoryProps) {
  const [candidate, setCandidate] = useState<RepositoryCandidate | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function inspectSelectedRepository() {
    setError(null);
    try {
      const selected = await openDialog({
        directory: true,
        multiple: false,
        title: "Open Git repository",
      });
      if (selected === null) return;

      setBusy(true);
      const outcome = await invoke<OpenRepositoryOutcome>("open_repository", {
        path: selected,
        trust: "requireConfirmation",
      });
      if (outcome.status === "trustRequired") {
        setCandidate(outcome.repository);
      } else {
        onOpened(outcome.repository);
      }
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  }

  async function trustRepository() {
    if (!candidate) return;
    setBusy(true);
    setError(null);
    try {
      const outcome = await invoke<OpenRepositoryOutcome>("open_repository", {
        path: candidate.rootPath,
        trust: "trust",
      });
      if (outcome.status !== "opened") {
        throw new Error("Repository still requires trust confirmation");
      }
      setCandidate(null);
      onOpened(outcome.repository);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex w-full max-w-sm flex-col items-center gap-3">
      <Button disabled={busy} onClick={inspectSelectedRepository} type="button">
        <FolderOpenIcon data-icon="inline-start" />
        {busy ? "Inspecting repository…" : "Open repository"}
      </Button>

      {error && !candidate ? <RepositoryError message={error} /> : null}

      <Dialog
        open={candidate !== null}
        onOpenChange={(open) => {
          if (!open && !busy) setCandidate(null);
        }}
      >
        <DialogContent showCloseButton={!busy}>
          <DialogHeader>
            <DialogTitle>Trust this repository?</DialogTitle>
            <DialogDescription>
              Mars will inspect local source and Git metadata to build project context. Agent commands and Git mutations remain separately permissioned.
            </DialogDescription>
          </DialogHeader>

          {candidate ? (
            <div className="flex flex-col gap-4">
              <div className="rounded-lg border bg-muted/30 p-3">
                <div className="flex items-center gap-2 font-medium">
                  <ShieldCheckIcon />
                  {candidate.name}
                </div>
                <p className="mt-2 break-all text-xs text-muted-foreground">{candidate.rootPath}</p>
              </div>
              <div className="flex flex-wrap gap-2">
                {candidate.capabilities.map((capability) => (
                  <Badge key={capability} variant="secondary">
                    {capabilityLabels[capability]}
                  </Badge>
                ))}
              </div>
            </div>
          ) : null}

          {error ? <RepositoryError message={error} /> : null}

          <DialogFooter>
            <DialogClose render={<Button disabled={busy} variant="outline" />}>Cancel</DialogClose>
            <Button disabled={busy} onClick={trustRepository} type="button">
              <ShieldCheckIcon data-icon="inline-start" />
              {busy ? "Opening…" : "Trust and open"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
