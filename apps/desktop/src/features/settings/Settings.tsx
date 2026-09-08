import type {
  AdapterProbe,
  ForgeConnection,
  GrantView,
  IndexStatus,
  StoragePaths,
} from "@mars/shared-types";
import { RefreshCwIcon, ShieldOffIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateLabel } from "@/shell/primitives";

export interface SettingsProps {
  agents: { id: string; name: string; probe: AdapterProbe | null; error: string | null }[];
  agentsState: ActivityState;
  connection: ForgeConnection | null;
  grants: GrantView[];
  grantsState: ActivityState;
  index: IndexStatus | null;
  storage: StoragePaths | null;
  repositoryName: string;
  onRevokeGrant: (grant: GrantView) => void;
  onReindex: () => void;
  onConnectGitHub: () => void;
  busyGrantId?: string | null;
}

function Section({
  children,
  description,
  title,
}: {
  children: React.ReactNode;
  description?: string;
  title: string;
}) {
  return (
    <section className="flex flex-col gap-2 border-b border-border px-3 py-3">
      <MetaLabel>{title}</MetaLabel>
      {description ? (
        <p className="text-[11px] leading-relaxed text-muted-foreground">{description}</p>
      ) : null}
      {children}
    </section>
  );
}

function Fact({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="w-28 shrink-0 text-[11px] text-faint">{label}</span>
      <span className="min-w-0 flex-1">{value}</span>
    </div>
  );
}

/** How a grant's scope reads, without leaking the shape of the union. */
function scopeSummary(grant: GrantView): string {
  const scope = grant.scope;
  switch (scope.kind) {
    case "repository":
      return "this repository";
    case "worktree":
      return `worktree ${scope.root}`;
    case "command":
      return `${scope.program} in ${scope.cwd}`;
    case "network_host":
      return scope.host;
    case "remote":
      return `${scope.remote} ${scope.branch}`;
    default:
      return "unknown scope";
  }
}

/**
 * What Mars is configured with, and the place every grant can be taken back.
 *
 * The trust screen promises this surface before it asks for anything, so it has
 * to be able to keep that promise: every standing grant is listed here — live
 * or spent — with who gave it and when, and one action to withdraw it. A grant
 * that cannot be found is a grant that cannot be revoked, so nothing is hidden
 * behind a task or a filter.
 */
export function Settings({
  agents,
  agentsState,
  busyGrantId,
  connection,
  grants,
  grantsState,
  index,
  onConnectGitHub,
  onReindex,
  onRevokeGrant,
  repositoryName,
  storage,
}: SettingsProps) {
  const active = grants.filter((grant) => grant.active);
  const spent = grants.filter((grant) => !grant.active);

  return (
    <div className="flex h-full min-h-0 flex-col overflow-y-auto" aria-label="Settings">
      <Section
        title="Agents"
        description="Each agent is an external program Mars starts. The version is what answered when it was last probed, not what is documented."
      >
        {agentsState === "loading" ? (
          <StateLabel state="loading" detail="probing" />
        ) : (
          <ul className="flex flex-col gap-1">
            {agents.map((agent) => (
              <li key={agent.id} className="flex flex-col gap-0.5">
                <div className="flex flex-wrap items-baseline gap-2">
                  <span className="text-[12px] text-foreground">{agent.name}</span>
                  <StateLabel
                    state={agent.probe ? "done" : "failed"}
                    detail={agent.probe ? agent.probe.version : "not found on PATH"}
                  />
                  {agent.probe ? (
                    <Mono className="ml-auto text-faint">
                      {agent.probe.supportsResume ? "resumable" : "no resume"}
                    </Mono>
                  ) : null}
                </div>
                {agent.probe ? (
                  <Mono className="text-faint">{agent.probe.executable}</Mono>
                ) : (
                  <span className="text-[11px] text-muted-foreground">
                    {agent.error ?? "Mars could not run this agent."}
                  </span>
                )}
              </li>
            ))}
          </ul>
        )}
        <p className="text-[11px] text-faint">
          Default role: implementer. A reviewer runs under a read-only ceiling that no decision
          during the run can widen.
        </p>
      </Section>

      <Section
        title="Credentials"
        description="Tokens are kept in the operating system's credential store, never in the Mars database and never in the repository."
      >
        {connection ? (
          <>
            <Fact label="GitHub" value={<Mono className="text-foreground">{connection.accountLogin ?? "connected"}</Mono>} />
            <Fact label="API" value={<Mono className="break-all text-faint">{connection.apiBase}</Mono>} />
            <Fact label="Stored as" value={<Mono className="break-all text-faint">{connection.credentialRef}</Mono>} />
          </>
        ) : (
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-[11px] text-muted-foreground">GitHub is not connected.</span>
            <button
              type="button"
              onClick={onConnectGitHub}
              className="border border-border px-2 py-0.5 text-[11px] outline-none hover:border-border-strong"
            >
              Connect GitHub
            </button>
          </div>
        )}
      </Section>

      <Section
        title="Permissions"
        description="Nothing is granted by default. Every effect an agent attempts is classified and asked about unless a grant below already covers it."
      >
        {grantsState === "loading" ? (
          <StateLabel state="loading" detail="reading grants" />
        ) : grants.length === 0 ? (
          <p className="text-[11px] text-muted-foreground">
            No grant has ever been made. Every effect will stop and ask.
          </p>
        ) : (
          <>
            <div className="flex items-baseline gap-2">
              <MetaLabel>Standing now</MetaLabel>
              <Mono className="text-faint">{active.length}</Mono>
            </div>
            {active.length === 0 ? (
              <p className="text-[11px] text-muted-foreground">Nothing stands. Everything asks.</p>
            ) : (
              <ul className="flex flex-col gap-1">
                {active.map((grant) => (
                  <li key={grant.id} className="flex flex-col gap-0.5 border border-border px-2 py-1">
                    <div className="flex flex-wrap items-baseline gap-2">
                      <Mono className="text-foreground">{grant.capability}</Mono>
                      {grant.pattern ? <Mono className="text-faint">{grant.pattern}</Mono> : null}
                      <Mono className="text-faint">{grant.decision}</Mono>
                      <button
                        type="button"
                        disabled={busyGrantId === grant.id}
                        onClick={() => onRevokeGrant(grant)}
                        className="ml-auto flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] text-destructive outline-none hover:border-destructive disabled:cursor-not-allowed disabled:text-faint"
                      >
                        <ShieldOffIcon className="size-3" />
                        Revoke
                      </button>
                    </div>
                    <span className="text-[11px] text-muted-foreground">
                      {scopeSummary(grant)} · granted by {grant.grantedBy} on {grant.grantedAt}
                      {grant.expiresAt ? ` · expires ${grant.expiresAt}` : ""}
                      {grant.usesRemaining === null ? "" : ` · ${grant.usesRemaining} use(s) left`}
                    </span>
                  </li>
                ))}
              </ul>
            )}

            {spent.length > 0 ? (
              <>
                <div className="flex items-baseline gap-2 pt-1">
                  <MetaLabel>No longer standing</MetaLabel>
                  <Mono className="text-faint">{spent.length}</Mono>
                </div>
                <ul className="flex flex-col gap-0.5">
                  {spent.map((grant) => (
                    <li key={grant.id} className="flex flex-wrap items-baseline gap-2">
                      <Mono className="text-faint">{grant.capability}</Mono>
                      <span className="text-[11px] text-faint">
                        {grant.revokedAt
                          ? `revoked by ${grant.revokedBy ?? "unknown"} on ${grant.revokedAt}`
                          : "expired or spent"}
                      </span>
                    </li>
                  ))}
                </ul>
              </>
            ) : null}
          </>
        )}
      </Section>

      <Section
        title="Indexing"
        description="The index is what makes questions about this code answerable. It is rebuilt on request, never silently."
      >
        <Fact label="Repository" value={<span className="text-[12px] text-foreground">{repositoryName}</span>} />
        {index ? (
          <>
            <Fact
              label="Read"
              value={
                <Mono className="text-muted-foreground">
                  {index.counts.files.toLocaleString()} files ·{" "}
                  {index.counts.symbols.toLocaleString()} symbols ·{" "}
                  {index.counts.chunks.toLocaleString()} chunks
                </Mono>
              }
            />
            <Fact
              label="Skipped"
              value={<Mono className="text-faint">{index.counts.ignoredPaths} ignored path(s)</Mono>}
            />
            <Fact
              label="Embeddings"
              value={
                <Mono className="text-faint">
                  {index.embeddingModel} · {index.embeddingDimensions}d
                </Mono>
              }
            />
            <Fact
              label="Freshness"
              value={
                <StateLabel
                  state={index.headMatchesIndex ? "done" : "stale"}
                  detail={index.headMatchesIndex ? "matches HEAD" : "the tree moved since the run"}
                />
              }
            />
          </>
        ) : null}
        <button
          type="button"
          onClick={onReindex}
          className={cn(
            "flex items-center gap-1 self-start border border-border px-2 py-0.5 text-[11px]",
            "outline-none hover:border-border-strong",
          )}
        >
          <RefreshCwIcon className="size-3" />
          Re-index now
        </button>
      </Section>

      <Section
        title="Where data lives"
        description="All of it is outside the repository. Mars writes nothing into your working tree except inside a task worktree you approved."
      >
        {storage ? (
          <>
            <Fact label="Index" value={<Mono className="break-all text-faint">{storage.database}</Mono>} />
            <Fact label="Worktrees" value={<Mono className="break-all text-faint">{storage.worktrees}</Mono>} />
            <Fact label="Attachments" value={<Mono className="break-all text-faint">{storage.artifacts}</Mono>} />
            <Fact label="Skills" value={<Mono className="break-all text-faint">{storage.skills}</Mono>} />
          </>
        ) : (
          <p className="text-[11px] text-faint">Paths are read when a repository is opened.</p>
        )}
      </Section>
    </div>
  );
}
