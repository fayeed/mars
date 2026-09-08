import {
  PROTOCOL_VERSION,
  type CommandRequest,
  type CommandResponse,
  type ProtocolVersion,
} from "@mars/protocol";
import type { ProjectSnapshot } from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

import { OpenRepository } from "@/features/projects/OpenRepository";
import { ShellWorkspace } from "@/features/workspace/ShellWorkspace";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateLabel } from "@/shell/primitives";
import { ShellProvider } from "@/shell/shell-store";
import { ThemeProvider } from "@/shell/theme";

type CoreConnection =
  | { phase: "connecting" }
  | { phase: "connected"; protocolVersion: ProtocolVersion }
  | { phase: "disconnected"; reason: string };

function formatVersion(version: ProtocolVersion) {
  return `${version.major}.${version.minor}.${version.patch}`;
}

function connectionState(connection: CoreConnection): ActivityState {
  if (connection.phase === "connected") return "done";
  return connection.phase === "connecting" ? "loading" : "failed";
}

function connectionDetail(connection: CoreConnection) {
  if (connection.phase === "connected") return `core v${formatVersion(connection.protocolVersion)}`;
  return connection.phase === "connecting" ? "core connecting" : "core offline";
}

function useCoreConnection() {
  const [connection, setConnection] = useState<CoreConnection>({ phase: "connecting" });

  useEffect(() => {
    let active = true;
    const request = {
      protocolVersion: PROTOCOL_VERSION,
      correlationId: crypto.randomUUID(),
      command: { type: "coreInfo" },
    } satisfies CommandRequest;

    invoke<CommandResponse>("core_info", { request })
      .then((response) => {
        if (!active) return;
        if (response.status === "success" && response.result.type === "coreInfo") {
          setConnection({ phase: "connected", protocolVersion: response.result.protocolVersion });
          return;
        }
        const reason =
          response.status === "failure" ? response.error.message : "Unexpected core response";
        setConnection({ phase: "disconnected", reason });
      })
      .catch((error: unknown) => {
        if (active) {
          const reason = error instanceof Error ? error.message : String(error);
          setConnection({ phase: "disconnected", reason });
        }
      });

    return () => {
      active = false;
    };
  }, []);

  return connection;
}

/**
 * Before a repository is trusted there is no shell to show: no tasks, no
 * worktree, no conversation to be live. This screen is the one place in Mars
 * that is not composed of shell regions.
 */
function OpenRepositoryScreen({
  connection,
  onOpened,
}: {
  connection: CoreConnection;
  onOpened: (project: ProjectSnapshot) => void;
}) {
  return (
    <main className="grid h-screen place-items-center bg-sunken text-foreground">
      <section className="w-[min(30rem,90vw)] border border-border bg-surface">
        <header className="flex items-center justify-between border-b border-border px-3 py-2">
          <span className="font-heading text-[13px] font-medium tracking-[0.3em]">
            M<span className="text-primary">/</span>RS
          </span>
          <StateLabel state={connectionState(connection)} detail={connectionDetail(connection)} />
        </header>

        <div className="flex flex-col gap-3 px-3 py-4">
          <MetaLabel>Repository</MetaLabel>
          <h1 className="font-heading text-xl tracking-tight">Open a repository to begin.</h1>
          <p className="text-[12px] leading-relaxed text-muted-foreground">
            Mars shows the exact canonical root and the read capabilities trust enables, then
            persists nothing until you confirm. Trust grants reading only — writes, commands and
            network stay separate.
          </p>
          {connection.phase === "disconnected" ? (
            <p className="border border-destructive/40 px-2 py-1 text-[11px] text-destructive">
              <Mono>{connection.reason}</Mono>
            </p>
          ) : null}
          <OpenRepository onOpened={onOpened} />
        </div>
      </section>
    </main>
  );
}

export function App() {
  const connection = useCoreConnection();
  const [project, setProject] = useState<ProjectSnapshot | null>(null);

  return (
    <ThemeProvider>
      {project ? (
        <ShellProvider>
          <ShellWorkspace
            key={project.repositoryId}
            connection={
              <StateLabel
                state={connectionState(connection)}
                detail={connectionDetail(connection)}
              />
            }
            onOpenProject={setProject}
            project={project}
          />
        </ShellProvider>
      ) : (
        <OpenRepositoryScreen connection={connection} onOpened={setProject} />
      )}
    </ThemeProvider>
  );
}
