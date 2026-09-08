import {
  PROTOCOL_VERSION,
  type CommandRequest,
  type CommandResponse,
  type ProtocolVersion,
} from "@mars/protocol";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

import { TrustFrame, TrustRepository, type TrustOutcome } from "@/features/setup/TrustRepository";
import { ShellWorkspace } from "@/features/workspace/ShellWorkspace";
import type { ActivityState } from "@/shell/activity-state";
import { StateLabel } from "@/shell/primitives";
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
 * worktree, no conversation to be live. The trust screen is the one place in
 * Mars that is not composed of shell regions, and it is the only gate between
 * opening the app and having a conversation.
 */
export function App() {
  const connection = useCoreConnection();
  const [opened, setOpened] = useState<TrustOutcome | null>(null);

  const status = (
    <StateLabel state={connectionState(connection)} detail={connectionDetail(connection)} />
  );

  return (
    <ThemeProvider>
      {opened ? (
        <ShellProvider>
          <ShellWorkspace
            key={opened.project.repositoryId}
            connection={status}
            indexing={opened.indexing}
            onOpenProject={(project) => setOpened({ project, indexing: opened.indexing })}
            project={opened.project}
          />
        </ShellProvider>
      ) : (
        <TrustFrame status={status}>
          <TrustRepository onOpened={setOpened} />
        </TrustFrame>
      )}
    </ThemeProvider>
  );
}
