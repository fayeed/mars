import type {
  CommandFailure,
  InspectedSkill,
  LocalSkill,
  SkillCandidate,
  SkillInstallation,
  SkillScope,
  SkillUpdate,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import {
  BadgeCheckIcon,
  DownloadIcon,
  RefreshCwIcon,
  SearchIcon,
  ShieldAlertIcon,
  TrashIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateBlock, StateLabel } from "@/shell/primitives";

export interface SkillsMarketplaceProps {
  repositoryId: string;
  /** The task open in the conversation, which a skill can be installed for. */
  taskId: string | null;
  taskTitle?: string;
  /** Called after anything is installed, changed or removed. */
  onChanged?: () => void;
}

type View = "discover" | "installed" | "machine";

/** Searches a person can start from without knowing a skill's name. */
export const TOPICS = [
  "testing",
  "frontend",
  "react",
  "typescript",
  "python",
  "rust",
  "documentation",
  "git",
  "security",
  "database",
  "devops",
  "design",
];

const SCOPE_LABEL: Record<SkillScope, string> = {
  task: "this task",
  workflow: "a workflow",
  project: "this repository",
  global: "every repository",
};

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

export function formatInstalls(installs: number): string {
  if (installs >= 1_000_000) return `${(installs / 1_000_000).toFixed(1)}M installs`;
  if (installs >= 1_000) return `${Math.round(installs / 1_000)}k installs`;
  return `${installs} install${installs === 1 ? "" : "s"}`;
}

/**
 * Skills, the way an editor's extension view works, with the one difference
 * that matters: a skill is someone else's instructions to an agent that acts
 * on this machine, so nothing is installed until its full text has been read
 * here, and what is installed is pinned to the exact version that was read.
 *
 * Discover searches skills.sh. Installing writes the pinned files once and
 * gives them to agents natively — Claude Code loads them as a plugin for each
 * run — at the scope chosen: one task, this repository, or everywhere.
 * Installed lists every installation that reaches this repository, with
 * switches for each agent, a check for a newer version, and a block that
 * stops a skill reaching any agent. On this machine lists the skills the CLIs
 * already load by themselves, from the repository and the home folder.
 */
export function SkillsMarketplace({
  onChanged,
  repositoryId,
  taskId,
  taskTitle,
}: SkillsMarketplaceProps) {
  const [view, setView] = useState<View>("discover");
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SkillCandidate[]>([]);
  const [searchState, setSearchState] = useState<ActivityState>("idle");
  const [searchError, setSearchError] = useState<string | null>(null);
  const [selected, setSelected] = useState<SkillCandidate | null>(null);
  const [inspected, setInspected] = useState<InspectedSkill | null>(null);
  const [inspectState, setInspectState] = useState<ActivityState>("idle");
  const [inspectError, setInspectError] = useState<string | null>(null);
  const [scope, setScope] = useState<SkillScope>(taskId ? "task" : "project");
  const [allowClaude, setAllowClaude] = useState(true);
  const [allowCodex, setAllowCodex] = useState(true);
  const [installing, setInstalling] = useState(false);
  const [installed, setInstalled] = useState<SkillInstallation[]>([]);
  const [installedState, setInstalledState] = useState<ActivityState>("loading");
  const [local, setLocal] = useState<LocalSkill[]>([]);
  const [localState, setLocalState] = useState<ActivityState>("loading");
  const [busyId, setBusyId] = useState<string | null>(null);
  const [updates, setUpdates] = useState<Record<string, SkillUpdate | "checking" | string>>({});
  const [expanded, setExpanded] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const loadInstalled = useCallback(() => {
    setInstalledState((current) => (current === "idle" ? "idle" : "loading"));
    return invoke<SkillInstallation[]>("repository_skills", { repositoryId })
      .then((items) => {
        setInstalled(items);
        setInstalledState("idle");
      })
      .catch(() => setInstalledState("failed"));
  }, [repositoryId]);

  useEffect(() => {
    void loadInstalled();
    invoke<LocalSkill[]>("local_skills", { repositoryId })
      .then((items) => {
        setLocal(items);
        setLocalState("idle");
      })
      .catch(() => setLocalState("failed"));
  }, [loadInstalled, repositoryId]);

  // Searching is debounced, and an answer to an older query is dropped rather
  // than allowed to replace a newer one.
  const latestQuery = useRef("");
  useEffect(() => {
    const text = query.trim();
    latestQuery.current = text;
    if (text.length < 2) {
      setResults([]);
      setSearchState("idle");
      return;
    }
    setSearchState("loading");
    const timer = setTimeout(() => {
      invoke<SkillCandidate[]>("search_skills", { query: text })
        .then((items) => {
          if (latestQuery.current !== text) return;
          setResults(items);
          setSearchState("idle");
          setSearchError(null);
        })
        .catch((caught: unknown) => {
          if (latestQuery.current !== text) return;
          setSearchState("failed");
          setSearchError(messageFor(caught));
        });
    }, 300);
    return () => clearTimeout(timer);
  }, [query]);

  async function inspect(candidate: SkillCandidate) {
    setSelected(candidate);
    setInspected(null);
    setInspectError(null);
    setNotice(null);
    setInspectState("loading");
    try {
      const skill = await invoke<InspectedSkill>("inspect_skill", { candidate });
      setInspected(skill);
      setInspectState("idle");
    } catch (caught) {
      setInspectError(messageFor(caught));
      setInspectState("failed");
    }
  }

  async function install() {
    if (!inspected) return;
    setInstalling(true);
    try {
      await invoke<SkillInstallation>("install_skill", {
        skillId: inspected.id,
        repositoryId,
        taskId,
        scope,
        allowClaude,
        allowCodex,
      });
      setNotice(
        `Installed ${inspected.name} for ${SCOPE_LABEL[scope]}, pinned at ${inspected.contentHash.slice(0, 12)}.`,
      );
      await loadInstalled();
      onChanged?.();
    } catch (caught) {
      setInspectError(messageFor(caught));
    } finally {
      setInstalling(false);
    }
  }

  async function act(id: string, action: () => Promise<unknown>) {
    setBusyId(id);
    try {
      await action();
      await loadInstalled();
      onChanged?.();
    } catch (caught) {
      setNotice(messageFor(caught));
    } finally {
      setBusyId(null);
    }
  }

  async function checkUpdate(installation: SkillInstallation) {
    setUpdates((current) => ({ ...current, [installation.id]: "checking" }));
    try {
      const update = await invoke<SkillUpdate>("check_skill_update", {
        installationId: installation.id,
      });
      setUpdates((current) => ({ ...current, [installation.id]: update }));
    } catch (caught) {
      setUpdates((current) => ({ ...current, [installation.id]: messageFor(caught) }));
    }
  }

  const installedKeys = useMemo(
    () => new Set(installed.map((installation) => `${installation.source}/${installation.name}`)),
    [installed],
  );

  const tabs: { id: View; label: string; count?: number }[] = [
    { id: "discover", label: "Discover" },
    { id: "installed", label: "Installed", count: installed.length },
    { id: "machine", label: "On this machine", count: local.length },
  ];

  return (
    <div className="flex h-full min-h-0 flex-col" data-slot="skills-marketplace">
      <div className="flex shrink-0 items-center gap-1 border-b border-border px-2 py-1.5">
        <div role="tablist" aria-label="Skill views" className="flex items-center gap-1">
          {tabs.map((tab) => (
            <button
              key={tab.id}
              type="button"
              role="tab"
              aria-selected={view === tab.id}
              onClick={() => setView(tab.id)}
              className={cn(
                "border px-2 py-0.5 text-[11px] outline-none",
                view === tab.id
                  ? "border-primary text-foreground"
                  : "border-border text-muted-foreground hover:border-border-strong",
              )}
            >
              {tab.label}
              {tab.count !== undefined ? <Mono className="ml-1 text-faint">{tab.count}</Mono> : null}
            </button>
          ))}
        </div>
        {notice ? (
          <span className="ml-auto truncate text-[11px] text-muted-foreground" role="status">
            {notice}
          </span>
        ) : null}
      </div>

      {view === "discover" ? (
        <div className="flex min-h-0 flex-1">
          <div className="flex w-[22rem] shrink-0 flex-col border-r border-border">
            <label className="flex items-center gap-2 border-b border-border px-2 py-1.5">
              <SearchIcon className="size-3.5 text-faint" />
              <span className="sr-only">Search skills</span>
              <input
                aria-label="Search skills"
                className="min-w-0 flex-1 bg-transparent text-[12px] text-foreground outline-none placeholder:text-faint"
                onChange={(event) => setQuery(event.target.value)}
                placeholder="Search skills.sh"
                value={query}
              />
            </label>
            {query.trim().length < 2 ? (
              <div className="flex flex-col gap-2 p-2">
                <MetaLabel>Start from a topic</MetaLabel>
                <div className="flex flex-wrap gap-1">
                  {TOPICS.map((topic) => (
                    <button
                      key={topic}
                      type="button"
                      onClick={() => setQuery(topic)}
                      className="border border-border px-1.5 py-0.5 text-[11px] text-muted-foreground outline-none hover:border-border-strong hover:text-foreground"
                    >
                      {topic}
                    </button>
                  ))}
                </div>
                <p className="text-[11px] leading-relaxed text-faint">
                  Nothing is installed until you have read the skill in full and chosen where it
                  applies.
                </p>
              </div>
            ) : searchState === "failed" ? (
              <StateBlock
                state="failed"
                title="skills.sh could not be searched"
                description={searchError ?? "Check the network and try again."}
              />
            ) : searchState === "loading" && results.length === 0 ? (
              <StateBlock state="loading" title="Searching skills.sh" />
            ) : results.length === 0 ? (
              <StateBlock state="idle" title="No skills match" description="Try other words." />
            ) : (
              <ul className="min-h-0 flex-1 overflow-auto" aria-label="Search results">
                {results.map((candidate) => {
                  const isInstalled = installedKeys.has(`${candidate.source}/${candidate.name}`);
                  return (
                    <li key={`${candidate.source}/${candidate.slug}`}>
                      <button
                        type="button"
                        onClick={() => void inspect(candidate)}
                        aria-current={selected === candidate ? "true" : undefined}
                        className={cn(
                          "flex w-full flex-col gap-0.5 border-b border-border px-2 py-1.5 text-left outline-none hover:bg-surface-raised focus-visible:bg-surface-raised",
                          selected === candidate && "bg-surface-raised",
                        )}
                      >
                        <span className="flex items-center gap-2">
                          <span className="text-[12px] text-foreground">{candidate.name}</span>
                          {isInstalled ? <Mono className="text-primary">installed</Mono> : null}
                        </span>
                        <span className="flex items-center gap-2">
                          <Mono className="truncate text-faint">{candidate.source}</Mono>
                          <Mono className="shrink-0 text-faint">
                            {formatInstalls(candidate.installs)}
                          </Mono>
                        </span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>

          <div className="flex min-w-0 flex-1 flex-col overflow-auto">
            {!selected ? (
              <StateBlock
                state="idle"
                title="Pick a skill to read it"
                description="Its whole SKILL.md is shown here, with where it came from and whether it matches what the registry published, before anything is installed."
              />
            ) : inspectState === "loading" ? (
              <StateBlock state="loading" title={`Downloading ${selected.name}`} />
            ) : inspectState === "failed" || !inspected ? (
              <StateBlock
                state="failed"
                title={`${selected.name} could not be read`}
                description={inspectError ?? "Nothing was installed."}
              />
            ) : (
              <SkillReview
                allowClaude={allowClaude}
                allowCodex={allowCodex}
                error={inspectError}
                installing={installing}
                onAllowClaude={setAllowClaude}
                onAllowCodex={setAllowCodex}
                onInstall={() => void install()}
                onScope={setScope}
                scope={scope}
                skill={inspected}
                taskTitle={taskId ? taskTitle ?? "this task" : null}
              />
            )}
          </div>
        </div>
      ) : null}

      {view === "installed" ? (
        <div className="min-h-0 flex-1 overflow-auto">
          {installedState === "loading" ? (
            <StateBlock state="loading" title="Reading installed skills" />
          ) : installedState === "failed" ? (
            <StateBlock state="failed" title="Installed skills could not be read" />
          ) : installed.length === 0 ? (
            <StateBlock
              state="idle"
              title="No skills installed"
              description="Find one in Discover. Nothing is installed by default, and nothing installs itself."
            />
          ) : (
            <ul aria-label="Installed skills">
              {installed.map((installation) => {
                const update = updates[installation.id];
                return (
                  <li
                    key={installation.id}
                    className="flex flex-col gap-1.5 border-b border-border px-3 py-2"
                  >
                    <div className="flex flex-wrap items-center gap-2">
                      <button
                        type="button"
                        className="text-[12px] text-foreground outline-none hover:underline focus-visible:underline"
                        onClick={() =>
                          setExpanded((current) =>
                            current === installation.id ? null : installation.id,
                          )
                        }
                        aria-expanded={expanded === installation.id}
                      >
                        {installation.name}
                      </button>
                      <Mono className="text-faint">{installation.source}</Mono>
                      <Mono className="text-faint">for {SCOPE_LABEL[installation.scope]}</Mono>
                      <Mono className="text-faint">pinned {installation.pinnedHash.slice(0, 12)}</Mono>
                      {installation.blocked ? (
                        <StateLabel state="failed" detail="blocked" />
                      ) : !installation.enabled ? (
                        <StateLabel state="idle" detail="switched off" />
                      ) : null}
                    </div>
                    {installation.description ? (
                      <p className="text-[11px] leading-relaxed text-muted-foreground">
                        {installation.description}
                      </p>
                    ) : null}
                    <div className="flex flex-wrap items-center gap-3">
                      <label className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                        <input
                          type="checkbox"
                          checked={installation.enabled}
                          disabled={installation.blocked || busyId === installation.id}
                          onChange={(event) =>
                            void act(installation.id, () =>
                              invoke("set_skill_enabled", {
                                installationId: installation.id,
                                enabled: event.target.checked,
                              }),
                            )
                          }
                        />
                        On
                      </label>
                      {(["claude", "codex"] as const).map((adapter) => (
                        <label
                          key={adapter}
                          className="flex items-center gap-1.5 text-[11px] text-muted-foreground"
                        >
                          <input
                            type="checkbox"
                            checked={adapter === "claude" ? installation.allowClaude : installation.allowCodex}
                            disabled={busyId === installation.id}
                            onChange={(event) =>
                              void act(installation.id, () =>
                                invoke("set_skill_adapter", {
                                  installationId: installation.id,
                                  adapter,
                                  allowed: event.target.checked,
                                }),
                              )
                            }
                          />
                          {adapter === "claude" ? "Claude" : "Codex"}
                        </label>
                      ))}
                      <button
                        type="button"
                        className="flex items-center gap-1 text-[11px] text-muted-foreground outline-none hover:text-foreground focus-visible:text-foreground"
                        disabled={update === "checking"}
                        onClick={() => void checkUpdate(installation)}
                      >
                        <RefreshCwIcon className="size-3" />
                        {update === "checking" ? "Checking" : "Check for update"}
                      </button>
                      <button
                        type="button"
                        className="flex items-center gap-1 text-[11px] text-muted-foreground outline-none hover:text-destructive focus-visible:text-destructive"
                        disabled={busyId === installation.id}
                        onClick={() =>
                          void act(installation.id, () =>
                            invoke("remove_skill_installation", {
                              installationId: installation.id,
                            }),
                          )
                        }
                      >
                        <TrashIcon className="size-3" />
                        Uninstall
                      </button>
                      {!installation.blocked ? (
                        <button
                          type="button"
                          className="flex items-center gap-1 text-[11px] text-muted-foreground outline-none hover:text-destructive focus-visible:text-destructive"
                          disabled={busyId === installation.id}
                          onClick={() =>
                            void act(installation.id, () =>
                              invoke("block_skill", { skillId: installation.skillId }),
                            )
                          }
                        >
                          <ShieldAlertIcon className="size-3" />
                          Block everywhere
                        </button>
                      ) : null}
                    </div>
                    {update && update !== "checking" ? (
                      typeof update === "string" ? (
                        <span className="text-[11px] text-destructive">{update}</span>
                      ) : update.changedFiles.length === 0 ? (
                        <StateLabel state="done" detail="up to date" />
                      ) : (
                        <div className="flex flex-wrap items-center gap-2">
                          <StateLabel
                            state="stale"
                            detail={`${update.changedFiles.length} file(s) changed: ${update.changedFiles.join(", ")}`}
                          />
                          <button
                            type="button"
                            className="border border-border px-1.5 py-0.5 text-[11px] text-foreground outline-none hover:border-border-strong"
                            onClick={() => {
                              setView("discover");
                              setScope(installation.scope);
                              void inspect({
                                name: installation.name,
                                slug: installation.slug,
                                source: installation.source,
                                installs: 0,
                              });
                            }}
                          >
                            Review the update
                          </button>
                        </div>
                      )
                    ) : null}
                    {expanded === installation.id ? (
                      <pre className="max-h-72 overflow-auto whitespace-pre-wrap border border-border bg-sunken p-2 font-mono text-[11px] leading-relaxed text-muted-foreground">
                        {installation.skillMarkdown}
                      </pre>
                    ) : null}
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      ) : null}

      {view === "machine" ? (
        <div className="min-h-0 flex-1 overflow-auto">
          {localState === "loading" ? (
            <StateBlock state="loading" title="Looking for skills on this machine" />
          ) : local.length === 0 ? (
            <StateBlock
              state="idle"
              title="No skills found on this machine"
              description="Mars looks in the repository's .claude/skills, .agents/skills and skills folders, and in ~/.claude/skills and ~/.codex/skills."
            />
          ) : (
            <ul aria-label="Skills on this machine">
              {local.map((skill) => (
                <li key={skill.path} className="flex flex-col gap-1 border-b border-border px-3 py-2">
                  <div className="flex flex-wrap items-center gap-2">
                    <button
                      type="button"
                      className="text-[12px] text-foreground outline-none hover:underline focus-visible:underline"
                      onClick={() =>
                        setExpanded((current) => (current === skill.path ? null : skill.path))
                      }
                      aria-expanded={expanded === skill.path}
                    >
                      {skill.name}
                    </button>
                    <Mono className="text-faint">
                      {skill.loadedBy.length > 0
                        ? `loaded by ${skill.loadedBy.join(" and ")} on its own`
                        : "loaded by no agent on its own"}
                    </Mono>
                  </div>
                  {skill.description ? (
                    <p className="text-[11px] leading-relaxed text-muted-foreground">
                      {skill.description}
                    </p>
                  ) : null}
                  <Mono className="truncate text-faint">{skill.path}</Mono>
                  {expanded === skill.path ? (
                    <pre className="max-h-72 overflow-auto whitespace-pre-wrap border border-border bg-sunken p-2 font-mono text-[11px] leading-relaxed text-muted-foreground">
                      {skill.skillMarkdown}
                    </pre>
                  ) : null}
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}
    </div>
  );
}

interface SkillReviewProps {
  skill: InspectedSkill;
  scope: SkillScope;
  onScope: (scope: SkillScope) => void;
  allowClaude: boolean;
  allowCodex: boolean;
  onAllowClaude: (allowed: boolean) => void;
  onAllowCodex: (allowed: boolean) => void;
  onInstall: () => void;
  installing: boolean;
  error: string | null;
  taskTitle: string | null;
}

function SkillReview({
  allowClaude,
  allowCodex,
  error,
  installing,
  onAllowClaude,
  onAllowCodex,
  onInstall,
  onScope,
  scope,
  skill,
  taskTitle,
}: SkillReviewProps) {
  const scopes: { id: SkillScope; label: string }[] = [
    ...(taskTitle ? [{ id: "task" as const, label: `Only ${taskTitle}` }] : []),
    { id: "project", label: "This repository" },
    { id: "global", label: "Every repository" },
  ];
  const blocked = skill.trustState === "blocked";
  return (
    <section aria-label={`Review ${skill.name}`} className="flex flex-col gap-3 p-3">
      <div className="flex flex-wrap items-baseline gap-2">
        <h2 className="text-[14px] font-medium text-foreground">{skill.name}</h2>
        <Mono className="text-faint">{skill.source}</Mono>
        {skill.installs > 0 ? <Mono className="text-faint">{formatInstalls(skill.installs)}</Mono> : null}
      </div>
      <div className="flex flex-wrap items-center gap-3">
        {skill.registryVerified ? (
          <span className="flex items-center gap-1 text-[11px] text-muted-foreground">
            <BadgeCheckIcon className="size-3.5" />
            Matches what skills.sh published
          </span>
        ) : (
          <span className="flex items-center gap-1 text-[11px] text-destructive">
            <ShieldAlertIcon className="size-3.5" />
            Does not match the registry's checksum — these may not be the files it lists
          </span>
        )}
        <Mono className="text-faint">pins {skill.contentHash.slice(0, 16)}</Mono>
        <Mono className="text-faint">{skill.files.length} file(s)</Mono>
      </div>
      {skill.changedFiles.length > 0 && skill.changedFiles.length < skill.files.length ? (
        <p className="text-[11px] text-primary">
          Changed since you last looked: {skill.changedFiles.join(", ")}
        </p>
      ) : null}
      <details className="text-[11px] text-muted-foreground">
        <summary className="cursor-pointer outline-none">Files</summary>
        <ul className="mt-1 flex flex-col">
          {skill.files.map((file) => (
            <li key={file}>
              <Mono className="text-faint">{file}</Mono>
            </li>
          ))}
        </ul>
      </details>
      <div className="flex flex-col gap-1">
        <MetaLabel>What agents will be told</MetaLabel>
        <pre className="max-h-[28rem] overflow-auto whitespace-pre-wrap border border-border bg-sunken p-2 font-mono text-[11px] leading-relaxed text-foreground">
          {skill.skillMarkdown}
        </pre>
      </div>
      {blocked ? (
        <StateBlock
          state="failed"
          title="This skill is blocked"
          description="It was blocked here before. It reaches no agent, and installing it again is refused."
        />
      ) : (
        <div className="flex flex-col gap-2 border-t border-border pt-2">
          <fieldset className="flex flex-wrap items-center gap-3">
            <legend className="sr-only">Install for</legend>
            <MetaLabel>Install for</MetaLabel>
            {scopes.map((option) => (
              <label key={option.id} className="flex items-center gap-1.5 text-[11px] text-foreground">
                <input
                  type="radio"
                  name="skill-scope"
                  checked={scope === option.id}
                  onChange={() => onScope(option.id)}
                />
                {option.label}
              </label>
            ))}
          </fieldset>
          <fieldset className="flex flex-wrap items-center gap-3">
            <legend className="sr-only">Agents</legend>
            <MetaLabel>Agents</MetaLabel>
            <label className="flex items-center gap-1.5 text-[11px] text-foreground">
              <input
                type="checkbox"
                checked={allowClaude}
                onChange={(event) => onAllowClaude(event.target.checked)}
              />
              Claude
            </label>
            <label className="flex items-center gap-1.5 text-[11px] text-foreground">
              <input
                type="checkbox"
                checked={allowCodex}
                onChange={(event) => onAllowCodex(event.target.checked)}
              />
              Codex
            </label>
          </fieldset>
          {error ? <p className="text-[11px] text-destructive">{error}</p> : null}
          <button
            type="button"
            disabled={installing || (!allowClaude && !allowCodex)}
            onClick={onInstall}
            className="flex w-fit items-center gap-1.5 border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:bg-transparent disabled:text-faint"
          >
            <DownloadIcon className="size-3.5" />
            {installing ? "Installing" : "Trust and install this version"}
          </button>
        </div>
      )}
    </section>
  );
}
