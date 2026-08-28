import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

interface ProtocolVersion {
  major: number;
  minor: number;
  patch: number;
}

interface CoreInfo {
  protocolVersion: ProtocolVersion;
}

type CoreConnection =
  | { phase: "connecting" }
  | { phase: "connected"; info: CoreInfo }
  | { phase: "disconnected"; reason: string };

const activities = [
  { code: "01", label: "Worktree", detail: "No workspace open" },
  { code: "02", label: "Agent run", detail: "No active assignment" },
  { code: "03", label: "Review queue", detail: "Nothing awaiting review" },
];

function formatVersion(version: ProtocolVersion) {
  return `${version.major}.${version.minor}.${version.patch}`;
}

export function App() {
  const [connection, setConnection] = useState<CoreConnection>({ phase: "connecting" });

  useEffect(() => {
    let active = true;

    invoke<CoreInfo>("core_info")
      .then((info) => {
        if (active) setConnection({ phase: "connected", info });
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

  const connected = connection.phase === "connected";

  return (
    <>
      <style>{styles}</style>
      <main className="workbench">
        <header className="masthead">
          <div className="wordmark" aria-label="Mars">
            M<span>/</span>RS
          </div>
          <div className="context">
            <span>LOCAL</span>
            <strong>UNTITLED WORKSPACE</strong>
          </div>
          <div className={`core-status core-status--${connection.phase}`} aria-live="polite">
            <span className="signal" aria-hidden="true" />
            <div>
              <b>{connected ? "CORE CONNECTED" : connection.phase === "connecting" ? "CONNECTING" : "CORE OFFLINE"}</b>
              <small>
                {connected
                  ? `PROTOCOL ${formatVersion(connection.info.protocolVersion)}`
                  : connection.phase === "connecting"
                    ? "NEGOTIATING PROTOCOL"
                    : "COMMAND BRIDGE UNAVAILABLE"}
              </small>
            </div>
          </div>
        </header>

        <aside className="rail" aria-label="Primary navigation">
          <button className="rail-action rail-action--active" type="button" aria-label="Tasks">T</button>
          <button className="rail-action" type="button" aria-label="Graph">G</button>
          <button className="rail-action" type="button" aria-label="Files">F</button>
          <span className="rail-spacer" />
          <button className="rail-action" type="button" aria-label="Settings">S</button>
        </aside>

        <section className="activity-panel" aria-labelledby="activity-title">
          <div className="section-heading">
            <p>OPERATIONS / NOW</p>
            <h1 id="activity-title">Activity</h1>
          </div>
          <div className="activity-list">
            {activities.map((activity) => (
              <article className="activity" key={activity.code}>
                <span>{activity.code}</span>
                <div>
                  <h2>{activity.label}</h2>
                  <p>{activity.detail}</p>
                </div>
                <i aria-hidden="true">—</i>
              </article>
            ))}
          </div>
        </section>

        <section className="canvas" aria-labelledby="canvas-title">
          <div className="canvas-index">MARS / 000</div>
          <div className="empty-state">
            <span className="crosshair" aria-hidden="true">+</span>
            <p>READY FOR ASSIGNMENT</p>
            <h2 id="canvas-title">Open a worktree to begin.</h2>
            <div className="rule" />
            <small>Tasks, agent activity, diffs, and review evidence will resolve here.</small>
          </div>
          <footer>
            <span>BRANCH —</span>
            <span>AGENTS 0</span>
            <span>QUEUE 0</span>
          </footer>
        </section>
      </main>
    </>
  );
}

const styles = `
  :root {
    color: #d8d5cc;
    background: #0b0c0c;
    font-family: "IBM Plex Mono", "Nimbus Mono PS", monospace;
    font-synthesis: none;
  }
  * { box-sizing: border-box; }
  body { margin: 0; min-width: 760px; min-height: 100vh; overflow: hidden; }
  button { font: inherit; }
  .workbench {
    --line: #292b2a;
    --muted: #777970;
    --paper: #d8d5cc;
    --signal: #ff5a1f;
    display: grid;
    grid-template: 64px calc(100vh - 64px) / 52px 320px minmax(390px, 1fr);
    min-height: 100vh;
    background: #0b0c0c;
  }
  .masthead {
    grid-column: 1 / -1;
    display: grid;
    grid-template-columns: 140px 1fr auto;
    align-items: stretch;
    border-bottom: 1px solid var(--line);
  }
  .wordmark {
    display: grid;
    place-items: center;
    border-right: 1px solid var(--line);
    font: 700 20px/1 "DIN Condensed", "Nimbus Sans Narrow", sans-serif;
    letter-spacing: .34em;
  }
  .wordmark span { color: var(--signal); }
  .context { display: flex; align-items: center; gap: 18px; padding: 0 24px; }
  .context span, .section-heading p, .canvas-index { color: var(--muted); font-size: 10px; letter-spacing: .16em; }
  .context strong { font-size: 11px; letter-spacing: .08em; }
  .core-status { min-width: 224px; display: flex; align-items: center; gap: 12px; border-left: 1px solid var(--line); padding: 0 22px; }
  .core-status b, .core-status small { display: block; }
  .core-status b { font-size: 10px; letter-spacing: .11em; }
  .core-status small { margin-top: 4px; color: var(--muted); font-size: 9px; }
  .signal { width: 7px; height: 7px; background: var(--signal); box-shadow: 0 0 0 3px #ff5a1f1c; }
  .core-status--connected .signal { background: #97c76a; box-shadow: 0 0 0 3px #97c76a1c; }
  .core-status--connecting .signal { animation: pulse 1.1s steps(2, end) infinite; }
  .rail { display: flex; flex-direction: column; align-items: center; border-right: 1px solid var(--line); padding: 13px 0; }
  .rail-action { width: 30px; height: 30px; margin-bottom: 7px; border: 0; color: var(--muted); background: transparent; font-size: 10px; cursor: pointer; }
  .rail-action:hover, .rail-action:focus-visible { color: var(--paper); outline: 1px solid var(--muted); }
  .rail-action--active { color: #0b0c0c; background: var(--paper); }
  .rail-spacer { flex: 1; }
  .activity-panel { border-right: 1px solid var(--line); background: #101111; }
  .section-heading { height: 140px; padding: 28px 26px; border-bottom: 1px solid var(--line); }
  .section-heading p { margin: 0 0 30px; }
  .section-heading h1 { margin: 0; font: 400 30px/1 "DIN Condensed", "Nimbus Sans Narrow", sans-serif; letter-spacing: -.03em; }
  .activity { display: grid; grid-template-columns: 35px 1fr 15px; min-height: 94px; padding: 21px 19px; border-bottom: 1px solid var(--line); }
  .activity > span, .activity > i { color: var(--muted); font-size: 9px; font-style: normal; }
  .activity h2 { margin: 0 0 8px; font-size: 11px; font-weight: 600; letter-spacing: .05em; }
  .activity p { margin: 0; color: var(--muted); font-size: 10px; }
  .canvas { position: relative; display: grid; place-items: center; background-image: linear-gradient(#171818 1px, transparent 1px), linear-gradient(90deg, #171818 1px, transparent 1px); background-size: 32px 32px; }
  .canvas::after { content: ""; position: absolute; inset: 0; background: radial-gradient(circle at 52% 46%, transparent, #0b0c0cd9 70%); pointer-events: none; }
  .canvas-index { position: absolute; z-index: 1; top: 22px; left: 24px; }
  .empty-state { z-index: 1; width: min(480px, 70%); }
  .crosshair { display: block; margin-bottom: 30px; color: var(--signal); font-size: 28px; font-weight: 200; }
  .empty-state p { color: var(--signal); font-size: 9px; letter-spacing: .2em; }
  .empty-state h2 { max-width: 420px; margin: 15px 0 30px; font: 400 42px/.98 "DIN Condensed", "Nimbus Sans Narrow", sans-serif; letter-spacing: -.04em; }
  .rule { width: 84px; height: 1px; margin-bottom: 17px; background: var(--paper); }
  .empty-state small { display: block; max-width: 390px; color: var(--muted); font-size: 10px; line-height: 1.7; }
  .canvas footer { position: absolute; z-index: 1; right: 0; bottom: 0; left: 0; display: flex; gap: 28px; padding: 12px 18px; border-top: 1px solid var(--line); color: var(--muted); background: #0b0c0ce8; font-size: 9px; }
  @keyframes pulse { 50% { opacity: .25; } }
  @media (prefers-reduced-motion: reduce) { .signal { animation: none; } }
`;
