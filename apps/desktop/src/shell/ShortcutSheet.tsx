import { KeyHint, MetaLabel } from "@/shell/primitives";
import { SHELL_BINDINGS, bindingHint } from "@/shell/keymap";
import { useModalFocus } from "@/shell/modal";

export interface ShortcutSheetProps {
  open: boolean;
  onClose: () => void;
}

/**
 * How the bindings group when a person is looking for one, which is not the
 * order they are declared in. Anything not named here falls into "Move".
 */
const GROUPS: { label: string; ids: string[] }[] = [
  {
    label: "Go",
    ids: ["go.conversation", "go.editor", "go.source-control", "open.palette"],
  },
  {
    label: "Move",
    ids: ["focus.composer", "focus.next", "focus.previous"],
  },
  {
    label: "Panels",
    ids: ["toggle.taskRail", "toggle.inspector", "toggle.utility"],
  },
];

/**
 * Every shortcut the shell has, from one of them.
 *
 * A keyboard-only product whose keyboard is undocumented is a keyboard-only
 * product for the person who wrote it. This is generated from `SHELL_BINDINGS`
 * rather than transcribed from it, so a binding cannot be added without
 * appearing here and a hint cannot drift from what the key actually does.
 */
export function ShortcutSheet({ onClose, open }: ShortcutSheetProps) {
  const { ref, onBackdropMouseDown } = useModalFocus<HTMLDivElement>({ active: open, onClose });

  if (!open) return null;

  const grouped = GROUPS.map((group) => ({
    label: group.label,
    bindings: group.ids
      .map((id) => SHELL_BINDINGS.find((candidate) => candidate.id === id))
      .filter((candidate): candidate is (typeof SHELL_BINDINGS)[number] => Boolean(candidate)),
  }));
  const listed = new Set(grouped.flatMap((group) => group.bindings.map((entry) => entry.id)));
  const rest = SHELL_BINDINGS.filter((candidate) => !listed.has(candidate.id));

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-sunken/70 p-8"
      onMouseDown={onBackdropMouseDown}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-label="Keyboard shortcuts"
        tabIndex={-1}
        className="flex max-h-[80vh] w-[min(34rem,92vw)] flex-col border border-border bg-surface outline-none"
      >
        <header className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-2">
          <MetaLabel className="text-foreground">Keyboard</MetaLabel>
          <button
            type="button"
            onClick={onClose}
            className="ml-auto flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] text-foreground outline-none hover:border-border-strong focus-visible:border-border-strong"
          >
            Close
            <KeyHint className="border-transparent" keys="Esc" />
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto">
          {[...grouped, { label: "Other", bindings: rest }]
            .filter((group) => group.bindings.length > 0)
            .map((group) => (
              <section key={group.label}>
                <div className="border-b border-border px-3 pt-2 pb-1">
                  <MetaLabel>{group.label}</MetaLabel>
                </div>
                <ul>
                  {group.bindings.map((entry) => (
                    <li
                      key={entry.id}
                      className="flex items-baseline gap-3 border-b border-border px-3 py-1.5 last:border-b-0"
                    >
                      <span className="min-w-0 flex-1 text-[12px] text-foreground">
                        {entry.label}
                      </span>
                      <KeyHint className="shrink-0" keys={entry.hint} />
                    </li>
                  ))}
                </ul>
              </section>
            ))}
        </div>

        <p className="shrink-0 border-t border-border px-3 py-1.5 text-[11px] text-muted-foreground">
          The composer is always{" "}
          <KeyHint className="border-transparent" keys={bindingHint("focus.composer")} /> away — it
          closes whatever is covering it on the way.
        </p>
      </div>
    </div>
  );
}
