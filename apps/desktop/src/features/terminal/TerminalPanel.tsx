import type { CommandFailure, ProjectSnapshot, TerminalOutput, TerminalSessionInfo } from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { AlertCircleIcon, PlayIcon, SquareIcon, TerminalSquareIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";

import "@xterm/xterm/css/xterm.css";

/**
 * A command to type into the shell for the person, such as an agent's own
 * sign-in. Typed rather than run behind their back, so what happens is on the
 * screen and theirs to interrupt. A new `id` types it again.
 */
export interface TerminalCommand {
  id: number;
  text: string;
}

interface TerminalPanelProps {
  project: ProjectSnapshot;
  command?: TerminalCommand | null;
  /**
   * Called once `command` has been typed, so the owner can let go of it. A
   * terminal mounted later, in another layout, must not type it a second time.
   */
  onCommandSent?: (id: number) => void;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

export function TerminalPanel({ command = null, onCommandSent, project }: TerminalPanelProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [started, setStarted] = useState(false);
  const [session, setSession] = useState<TerminalSessionInfo | null>(null);
  const sessionIdRef = useRef<string | null>(null);
  const pendingCommandRef = useRef<TerminalCommand | null>(null);
  const onCommandSentRef = useRef(onCommandSent);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    onCommandSentRef.current = onCommandSent;
  }, [onCommandSent]);

  /** Types the waiting command once there is a shell to type it into. */
  const flushCommand = useCallback(() => {
    const sessionId = sessionIdRef.current;
    const pending = pendingCommandRef.current;
    if (!sessionId || pending === null) return;
    pendingCommandRef.current = null;
    onCommandSentRef.current?.(pending.id);
    void invoke("write_terminal", { sessionId, data: `${pending.text}\r` }).catch((caught: unknown) =>
      setError(messageFor(caught)),
    );
  }, []);

  // Each request is a new object, so the same text asked for twice is typed
  // twice; a re-render that keeps the same request types nothing.
  useEffect(() => {
    if (!command) return;
    pendingCommandRef.current = command;
    if (sessionIdRef.current) flushCommand();
    else setStarted(true);
  }, [command, flushCommand]);

  useEffect(() => {
    if (!started || !containerRef.current) return;

    let disposed = false;
    let unlisten: (() => void) | undefined;
    const pendingOutput: TerminalOutput[] = [];
    const terminal = new Terminal({
      convertEol: false,
      cursorBlink: true,
      fontFamily: "Geist Mono, ui-monospace, monospace",
      fontSize: 12,
      scrollback: 5_000,
      theme: {
        background: "#11110f",
        foreground: "#e8e5dc",
        cursor: "#d6ff5f",
        selectionBackground: "#d6ff5f33",
      },
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(containerRef.current);
    fit.fit();

    const input = terminal.onData((data) => {
      const sessionId = sessionIdRef.current;
      if (sessionId) {
        void invoke("write_terminal", { sessionId, data }).catch((caught: unknown) => {
          if (!disposed) setError(messageFor(caught));
        });
      }
    });
    const resize = new ResizeObserver(() => {
      fit.fit();
      const sessionId = sessionIdRef.current;
      if (sessionId) {
        void invoke("resize_terminal", {
          sessionId,
          columns: terminal.cols,
          rows: terminal.rows,
        }).catch((caught: unknown) => {
          if (!disposed) setError(messageFor(caught));
        });
      }
    });
    resize.observe(containerRef.current);

    void (async () => {
      try {
        unlisten = await listen<TerminalOutput>("terminal-output", ({ payload }) => {
          if (sessionIdRef.current === null) {
            pendingOutput.push(payload);
          } else if (payload.sessionId === sessionIdRef.current) {
            terminal.write(Uint8Array.from(payload.data));
          }
        });
        const opened = await invoke<TerminalSessionInfo>("spawn_terminal", {
          repositoryId: project.repositoryId,
          columns: terminal.cols,
          rows: terminal.rows,
        });
        if (disposed) {
          await invoke("cancel_terminal", { sessionId: opened.sessionId });
          return;
        }
        sessionIdRef.current = opened.sessionId;
        for (const output of pendingOutput) {
          if (output.sessionId === opened.sessionId) terminal.write(Uint8Array.from(output.data));
        }
        setSession(opened);
        terminal.focus();
        flushCommand();
      } catch (caught) {
        if (!disposed) {
          setError(messageFor(caught));
          setStarted(false);
        }
      }
    })();

    return () => {
      disposed = true;
      resize.disconnect();
      input.dispose();
      unlisten?.();
      const sessionId = sessionIdRef.current;
      sessionIdRef.current = null;
      if (sessionId) void invoke("cancel_terminal", { sessionId });
      terminal.dispose();
    };
  }, [flushCommand, project.repositoryId, started]);

  async function stop() {
    if (!session) {
      setStarted(false);
      return;
    }
    try {
      await invoke("cancel_terminal", { sessionId: session.sessionId });
      sessionIdRef.current = null;
      setSession(null);
      setStarted(false);
    } catch (caught) {
      setError(messageFor(caught));
    }
  }

  return (
    <section className="flex h-full min-h-0 flex-col bg-[#11110f] text-[#e8e5dc]" aria-label="Terminal">
      <header className="flex h-10 shrink-0 items-center gap-2 border-b border-white/10 px-3">
        <TerminalSquareIcon className="size-4 text-primary" />
        <p className="text-[11px] font-medium uppercase tracking-wider">Terminal</p>
        <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-white/45">
          {session?.cwd ?? project.rootPath}
        </span>
        {started ? (
          <Button disabled={!session} onClick={() => void stop()} size="sm" type="button" variant="ghost">
            <SquareIcon data-icon="inline-start" /> Stop
          </Button>
        ) : null}
      </header>

      {error ? (
        <Alert className="m-3 w-auto" variant="destructive">
          <AlertCircleIcon />
          <AlertTitle>Terminal unavailable</AlertTitle>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      ) : null}

      {!started ? (
        <div className="grid min-h-0 flex-1 place-items-center">
          <div className="flex max-w-sm flex-col items-center gap-3 text-center">
            <p className="text-xs text-white/55">Launch your default shell in this trusted repository.</p>
            <Button
              onClick={() => {
                setError(null);
                setStarted(true);
              }}
              size="sm"
              type="button"
            >
              <PlayIcon data-icon="inline-start" /> Start terminal
            </Button>
          </div>
        </div>
      ) : (
        <div className="min-h-0 flex-1 p-2" ref={containerRef} />
      )}
    </section>
  );
}
