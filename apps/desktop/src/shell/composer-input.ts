export type TriggerKind = "mention" | "skill";

export interface ComposerTrigger {
  kind: TriggerKind;
  /** Index of the `@` or `/` character itself. */
  start: number;
  /** Text typed after the trigger character, up to the caret. */
  query: string;
}

/**
 * Decides whether the caret is inside an `@` mention or a `/` skill.
 *
 * `@` opens after any whitespace, because mentioning a file mid-sentence is
 * normal. `/` only opens at the start of a line — otherwise every repository
 * path someone types would open the skill menu.
 */
export function readTrigger(value: string, caret: number): ComposerTrigger | null {
  const upToCaret = value.slice(0, caret);

  for (let index = upToCaret.length - 1; index >= 0; index -= 1) {
    const character = upToCaret[index];
    if (character === "\n") return null;
    if (character === " " || character === "\t") return null;
    if (character !== "@" && character !== "/") continue;

    const previous = index === 0 ? "" : upToCaret[index - 1];
    const query = upToCaret.slice(index + 1);

    if (character === "@") {
      if (index === 0 || previous === " " || previous === "\t" || previous === "\n") {
        return { kind: "mention", start: index, query };
      }
      return null;
    }

    if (index === 0 || previous === "\n") {
      return { kind: "skill", start: index, query };
    }
    // A `/` inside a word is a path separator. Keep scanning: an `@` may still
    // open before it, as in `@src/shell/Composer.tsx`.
  }

  return null;
}

export interface CompletionResult {
  value: string;
  caret: number;
}

/**
 * Replaces the trigger and its query with the chosen token, leaving exactly one
 * space after it — completing mid-sentence must not double the space that is
 * already there.
 */
export function applyCompletion(
  value: string,
  trigger: ComposerTrigger,
  token: string,
): CompletionResult {
  const prefix = value.slice(0, trigger.start);
  const suffix = value.slice(trigger.start + 1 + trigger.query.length);
  const spaced = /^\s/.test(suffix);
  const inserted = spaced ? token : `${token} `;
  return { value: `${prefix}${inserted}${suffix}`, caret: prefix.length + inserted.length + (spaced ? 1 : 0) };
}

/** Case-insensitive subsequence-free, substring match over label and detail. */
export function filterCandidates<T extends { label?: string; name?: string; detail?: string }>(
  candidates: readonly T[],
  query: string,
  limit = 8,
): T[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return candidates.slice(0, limit);
  return candidates
    .filter((candidate) => {
      const haystack = `${candidate.label ?? candidate.name ?? ""} ${candidate.detail ?? ""}`;
      return haystack.toLowerCase().includes(needle);
    })
    .slice(0, limit);
}
