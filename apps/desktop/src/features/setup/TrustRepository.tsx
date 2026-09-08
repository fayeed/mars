import type {
  CommandFailure,
  OpenRepositoryOutcome,
  ProjectSnapshot,
  RepositoryCandidate,
  RepositoryCapability,
  RepositoryPreview,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { FolderOpenIcon, ShieldCheckIcon } from "lucide-react";
import { useCallback, useState, type ReactNode } from "react";

import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateBlock } from "@/shell/primitives";

/**
 * What trust actually turns on. These are read capabilities; the shape of the
 * screen depends on that staying true, so they are named here rather than
 * inferred from a badge list.
 */
const CAPABILITY_LABEL: Record<RepositoryCapability, string> = {
  readFiles: "Read source files",
  indexCode: "Build a local code index",
  observeGit: "Read Git metadata",
};

/**
 * The other half of the posture, and the reason trust is safe to give. None of
 * these follow from trusting a repository: each one stops an agent and asks,
 * per task, through the permission broker.
 */
const WITHHELD = [
  { label: "Write files", detail: "only inside a task worktree, only in the approved scope" },
  { label: "Run commands", detail: "classified, then asked — destructive ones are called out" },
  { label: "Network access", detail: "asked per host" },
  { label: "Push to a remote", detail: "asked per remote and branch" },
];

export type IndexingChoice = "requested" | "deferred";

export interface TrustOutcome {
  project: ProjectSnapshot;
  /** `deferred` opened the repository without starting an index. */
  indexing: IndexingChoice;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

function shortOid(oid: string) {
  return oid.slice(0, 7);
}

/** A number a person can read at a glance, or an em dash when it is not known. */
function count(value: number | null) {
  return value === null ? "—" : value.toLocaleString();
}

function Section({ children, title }: { children: ReactNode; title: string }) {
  return (
    <section className="flex flex-col gap-2 border-t border-border px-3 py-3">
      <MetaLabel>{title}</MetaLabel>
      {children}
    </section>
  );
}

/** A label and its machine-authored value, on one line. */
function Fact({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="w-24 shrink-0 text-[11px] text-faint">{label}</span>
      <span className="min-w-0 flex-1">{value}</span>
    </div>
  );
}

/**
 * The only gate before the conversation.
 *
 * Everything on this screen is read before anything is written: the repository
 * is inspected and previewed without being trusted, so the decision is made
 * against what is actually on disk rather than against a promise. Both ways out
 * are real — read-only opens the repository and indexes nothing — and the
 * screen says where the decision is undone before it is made.
 */
export function TrustRepository({ onOpened }: { onOpened: (outcome: TrustOutcome) => void }) {
  const [candidate, setCandidate] = useState<RepositoryCandidate | null>(null);
  const [preview, setPreview] = useState<RepositoryPreview | null>(null);
  const [previewState, setPreviewState] = useState<ActivityState>("idle");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const inspect = useCallback(async () => {
    setError(null);
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: "Open Git repository",
    });
    if (selected === null) return;

    setBusy(true);
    try {
      const outcome = await invoke<OpenRepositoryOutcome>("open_repository", {
        path: selected,
        trust: "requireConfirmation",
      });
      if (outcome.status !== "trustRequired") {
        // Already trusted in an earlier session; there is nothing to gate.
        onOpened({ project: outcome.repository, indexing: "requested" });
        return;
      }
      setCandidate(outcome.repository);
      setPreviewState("loading");
      // Reading the repository is the disclosure this screen exists to make, so
      // it is a separate step with its own state: a preview that fails must not
      // look like a repository that is empty.
      invoke<RepositoryPreview>("preview_repository", { path: outcome.repository.rootPath })
        .then((result) => {
          setPreview(result);
          setPreviewState("done");
        })
        .catch(() => setPreviewState("failed"));
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }, [onOpened]);

  async function open(indexing: IndexingChoice) {
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
      onOpened({ project: outcome.repository, indexing });
    } catch (caught) {
      setError(messageFor(caught));
      setBusy(false);
    }
  }

  if (!candidate) {
    return (
      <div className="flex flex-col gap-3 px-3 py-4">
        <MetaLabel>Repository</MetaLabel>
        <h1 className="font-heading text-xl tracking-tight">Open a repository to begin.</h1>
        <p className="text-[12px] leading-relaxed text-muted-foreground">
          Mars reads the repository before it trusts it, and shows you what it found. Nothing is
          written and no index is built until you decide.
        </p>
        {error ? (
          <p className="border border-destructive/40 px-2 py-1 text-[11px] text-destructive">
            <Mono>{error}</Mono>
          </p>
        ) : null}
        <button
          type="button"
          disabled={busy}
          onClick={() => void inspect()}
          className="flex items-center justify-center gap-1.5 self-start border border-border bg-primary px-2.5 py-1.5 text-[12px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:border-border disabled:bg-transparent disabled:text-faint"
        >
          <FolderOpenIcon className="size-3.5" />
          {busy ? "Inspecting…" : "Open repository"}
        </button>
      </div>
    );
  }

  return (
    <div className="flex max-h-[82vh] flex-col overflow-y-auto">
      <div className="flex flex-col gap-1 px-3 py-3">
        <MetaLabel>Trust this repository?</MetaLabel>
        <h1 className="font-heading text-lg tracking-tight">{candidate.name}</h1>
        <Mono className="break-all text-faint">{candidate.rootPath}</Mono>
      </div>

      <Section title="What is here">
        {previewState === "loading" ? (
          <StateBlock
            className="h-auto py-4"
            state="loading"
            title="Reading the repository"
            description="Resolving the remote, the branch, the commit, and how much of this tree is indexable."
          />
        ) : previewState === "failed" || !preview ? (
          <StateBlock
            className="h-auto py-4"
            state="failed"
            title="The repository could not be read"
            description="Nothing was changed. Trust is still yours to give, but this screen cannot tell you what you would be trusting."
          />
        ) : (
          <div className="flex flex-col gap-1.5">
            <Fact
              label="Remote"
              value={
                preview.remote ? (
                  <Mono className="break-all text-muted-foreground">{preview.remote}</Mono>
                ) : (
                  <span className="text-[11px] text-faint">no remote configured</span>
                )
              }
            />
            <Fact
              label="Branch"
              value={
                preview.branch ? (
                  <Mono className="text-muted-foreground">{preview.branch}</Mono>
                ) : (
                  <span className="text-[11px] text-faint">could not be read</span>
                )
              }
            />
            <Fact
              label="Commit"
              value={
                preview.headOid ? (
                  <span className="flex min-w-0 items-baseline gap-2">
                    <Mono className="text-muted-foreground">{shortOid(preview.headOid)}</Mono>
                    <span className="min-w-0 flex-1 truncate text-[11px] text-faint">
                      {preview.headSummary ?? ""}
                    </span>
                  </span>
                ) : (
                  <span className="text-[11px] text-faint">no commits yet</span>
                )
              }
            />
          </div>
        )}
      </Section>

      {preview ? (
        <>
          <Section title="What Mars will index">
            <p className="text-[12px] text-foreground">
              <Mono className="text-[12px] text-foreground">{count(preview.indexableFiles)}</Mono> of{" "}
              <Mono className="text-[12px] text-muted-foreground">
                {count(preview.trackedFiles)}
              </Mono>{" "}
              tracked files
            </p>
            <ul className="flex flex-col gap-1">
              {preview.skipped.map((rule) => (
                <li key={rule.label} className="flex items-baseline gap-2">
                  <span className="w-24 shrink-0 text-[11px] text-faint">{rule.label}</span>
                  <span className="min-w-0 flex-1 text-[11px] text-muted-foreground">
                    {rule.detail}
                  </span>
                </li>
              ))}
            </ul>
          </Section>

          <Section title="Where it stores what it learns">
            <Fact label="Index" value={<Mono className="break-all text-faint">{preview.storage.database}</Mono>} />
            <Fact label="Worktrees" value={<Mono className="break-all text-faint">{preview.storage.worktrees}</Mono>} />
            <Fact label="Attachments" value={<Mono className="break-all text-faint">{preview.storage.artifacts}</Mono>} />
            <Fact label="Skills" value={<Mono className="break-all text-faint">{preview.storage.skills}</Mono>} />
            <p className="text-[11px] text-muted-foreground">
              All outside the repository. Mars writes nothing into your working tree except inside a
              task worktree you approve.
            </p>
          </Section>
        </>
      ) : null}

      <Section title="What trust grants">
        <ul className="flex flex-col gap-1">
          {candidate.capabilities.map((capability) => (
            <li key={capability} className="flex items-center gap-2 text-[12px] text-foreground">
              <ShieldCheckIcon className="size-3.5 shrink-0 text-primary" />
              {CAPABILITY_LABEL[capability]}
            </li>
          ))}
        </ul>
        <MetaLabel className="pt-2">And what it does not</MetaLabel>
        <ul className="flex flex-col gap-1">
          {WITHHELD.map((item) => (
            <li key={item.label} className="flex items-baseline gap-2">
              <span className="w-24 shrink-0 text-[11px] text-muted-foreground">{item.label}</span>
              <span className="min-w-0 flex-1 text-[11px] text-faint">{item.detail}</span>
            </li>
          ))}
        </ul>
        <p className="text-[11px] text-muted-foreground">
          Each of those stops the agent and asks, per task. Trust does not widen them.
        </p>
      </Section>

      {error ? (
        <p className="mx-3 mb-2 border border-destructive/40 px-2 py-1 text-[11px] text-destructive">
          <Mono>{error}</Mono>
        </p>
      ) : null}

      <div className="sticky bottom-0 flex flex-wrap items-center gap-2 border-t border-border bg-surface px-3 py-2">
        <span className="min-w-0 flex-1 text-[11px] text-faint">
          Revoke this in Settings › Repository trust. Revoking drops the index and stops every
          standing grant.
        </span>
        <button
          type="button"
          disabled={busy}
          onClick={() => void open("deferred")}
          className="border border-border px-2 py-1 text-[11px] text-foreground outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
        >
          Open read-only
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => void open("requested")}
          className="flex items-center gap-1.5 border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:border-border disabled:bg-transparent disabled:text-faint"
        >
          <ShieldCheckIcon className="size-3.5" />
          {busy ? "Opening…" : "Trust and index"}
        </button>
      </div>
    </div>
  );
}

/** The connection line the shell would normally own, for the one screen without a shell. */
export function TrustFrame({ children, status }: { children: ReactNode; status: ReactNode }) {
  return (
    <main className="grid h-screen place-items-center bg-sunken text-foreground">
      <section className="w-[min(34rem,92vw)] border border-border bg-surface">
        <header className="flex items-center justify-between border-b border-border px-3 py-2">
          <span className="font-heading text-[13px] font-medium tracking-[0.3em]">
            M<span className="text-primary">/</span>RS
          </span>
          {status}
        </header>
        {children}
      </section>
    </main>
  );
}
