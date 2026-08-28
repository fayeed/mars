import { invoke } from "@tauri-apps/api/core";
import {
  PROTOCOL_VERSION,
  type CommandRequest,
  type CommandResponse,
  type ProtocolVersion,
} from "@mars/protocol";
import type { ProjectSnapshot } from "@mars/shared-types";
import {
  BlocksIcon,
  BrainCircuitIcon,
  Code2Icon,
  FolderGit2Icon,
  GitBranchIcon,
  Settings2Icon,
  TerminalSquareIcon,
} from "lucide-react";
import { lazy, Suspense, useEffect, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Separator } from "@/components/ui/separator";
import { OpenRepository } from "@/features/projects/OpenRepository";

const RepositoryWorkspace = lazy(() =>
  import("@/features/workspace/RepositoryWorkspace").then((module) => ({
    default: module.RepositoryWorkspace,
  })),
);

type CoreConnection =
  | { phase: "connecting" }
  | { phase: "connected"; protocolVersion: ProtocolVersion }
  | { phase: "disconnected"; reason: string };

const activities = [
  { code: "01", label: "Worktree", detail: "No workspace open" },
  { code: "02", label: "Agent run", detail: "No active assignment" },
  { code: "03", label: "Review queue", detail: "Nothing awaiting review" },
];

const navigation = [
  { label: "Tasks", icon: BlocksIcon, active: true },
  { label: "Project Brain", icon: BrainCircuitIcon },
  { label: "Explorer", icon: FolderGit2Icon },
  { label: "Terminal", icon: TerminalSquareIcon },
];

function formatVersion(version: ProtocolVersion) {
  return `${version.major}.${version.minor}.${version.patch}`;
}

function connectionLabel(connection: CoreConnection) {
  if (connection.phase === "connected") {
    return `Core · v${formatVersion(connection.protocolVersion)}`;
  }

  return connection.phase === "connecting" ? "Core · connecting" : "Core · offline";
}

export function App() {
  const [connection, setConnection] = useState<CoreConnection>({ phase: "connecting" });
  const [project, setProject] = useState<ProjectSnapshot | null>(null);

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
          setConnection({
            phase: "connected",
            protocolVersion: response.result.protocolVersion,
          });
          return;
        }
        const reason = response.status === "failure" ? response.error.message : "Unexpected core response";
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

  const statusVariant =
    connection.phase === "connected"
      ? "default"
      : connection.phase === "connecting"
        ? "outline"
        : "destructive";

  return (
    <main className="grid min-h-screen min-w-[760px] grid-cols-[52px_320px_minmax(390px,1fr)] grid-rows-[64px_calc(100vh-64px)] overflow-hidden bg-background text-foreground">
      <header className="col-span-full grid grid-cols-[140px_1fr_auto] border-b">
        <div className="grid place-items-center border-r font-heading text-lg font-semibold tracking-[0.32em]">
          M<span className="text-primary">/</span>RS
        </div>

        <div className="flex items-center gap-4 px-6 text-xs uppercase tracking-[0.12em]">
          <span className="text-muted-foreground">Local</span>
          <strong className="font-medium">{project?.name ?? "Untitled workspace"}</strong>
        </div>

        <div className="flex min-w-56 items-center border-l px-5" aria-live="polite">
          <Badge variant={statusVariant}>{connectionLabel(connection)}</Badge>
        </div>
      </header>

      <aside className="flex flex-col items-center gap-2 border-r py-3" aria-label="Primary navigation">
        {navigation.map(({ active, icon: Icon, label }) => (
          <Button
            aria-label={label}
            aria-current={active ? "page" : undefined}
            key={label}
            size="icon"
            type="button"
            variant={active ? "secondary" : "ghost"}
          >
            <Icon data-icon="inline-start" />
          </Button>
        ))}
        <div className="flex-1" />
        <Button aria-label="Settings" size="icon" type="button" variant="ghost">
          <Settings2Icon data-icon="inline-start" />
        </Button>
      </aside>

      {project ? (
        <Suspense
          fallback={
            <section className="col-span-2 grid place-items-center text-xs text-muted-foreground">
              Loading repository workspace…
            </section>
          }
        >
          <RepositoryWorkspace project={project} />
        </Suspense>
      ) : (
        <>
          <section className="border-r bg-card" aria-labelledby="activity-title">
            <div className="flex h-[140px] flex-col justify-between border-b px-6 py-7">
              <p className="text-[10px] uppercase tracking-[0.16em] text-muted-foreground">Operations / now</p>
              <h1 className="font-heading text-3xl tracking-tight" id="activity-title">Activity</h1>
            </div>

            <div className="flex flex-col">
              {activities.map((activity) => (
                <article className="grid min-h-24 grid-cols-[35px_1fr] items-start px-5 py-5" key={activity.code}>
                  <span className="text-[10px] text-muted-foreground">{activity.code}</span>
                  <div className="flex flex-col gap-2">
                    <h2 className="text-xs font-medium tracking-wide">{activity.label}</h2>
                    <p className="truncate text-xs text-muted-foreground">{activity.detail}</p>
                  </div>
                  <Separator className="col-span-full mt-5" />
                </article>
              ))}
            </div>
          </section>

          <section className="brain-grid relative grid place-items-center" aria-labelledby="canvas-title">
            <span className="absolute left-6 top-5 text-[10px] uppercase tracking-[0.16em] text-muted-foreground">
              Mars / 000
            </span>

            <Empty className="max-w-lg border-none">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <Code2Icon />
                </EmptyMedia>
                <p className="text-[10px] uppercase tracking-[0.2em] text-primary">Ready for assignment</p>
                <EmptyTitle className="font-heading text-4xl tracking-tight" id="canvas-title">
                  Open a repository to begin.
                </EmptyTitle>
                <EmptyDescription>
                  Choose a local Git repository. Mars will show exactly which read capabilities trust enables.
                </EmptyDescription>
              </EmptyHeader>
              <OpenRepository onOpened={setProject} />
            </Empty>

            <footer className="absolute inset-x-0 bottom-0 flex gap-7 border-t bg-background/90 px-5 py-3 text-[10px] uppercase tracking-wide text-muted-foreground backdrop-blur-sm">
              <span className="inline-flex items-center gap-2"><GitBranchIcon /> Branch —</span>
              <span>Agents 0</span>
              <span>Queue 0</span>
            </footer>
          </section>
        </>
      )}
    </main>
  );
}
