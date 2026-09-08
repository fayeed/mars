import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

export type ThemeMode = "dark" | "light" | "system";

const THEME_STORAGE_KEY = "mars.shell.theme";
const THEME_MODES: readonly ThemeMode[] = ["dark", "light", "system"];

export function readStoredTheme(storage: Pick<Storage, "getItem"> | undefined): ThemeMode {
  try {
    const raw = storage?.getItem(THEME_STORAGE_KEY);
    return THEME_MODES.includes(raw as ThemeMode) ? (raw as ThemeMode) : "dark";
  } catch {
    return "dark";
  }
}

export function resolveTheme(mode: ThemeMode, prefersDark: boolean): "dark" | "light" {
  if (mode === "system") return prefersDark ? "dark" : "light";
  return mode;
}

/** The rail toggle walks dark → light → system → dark. */
export function nextThemeMode(mode: ThemeMode): ThemeMode {
  return THEME_MODES[(THEME_MODES.indexOf(mode) + 1) % THEME_MODES.length];
}

interface ThemeContextValue {
  mode: ThemeMode;
  resolved: "dark" | "light";
  setMode: (mode: ThemeMode) => void;
  toggle: () => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({
  children,
  storage,
}: {
  children: ReactNode;
  storage?: Pick<Storage, "getItem" | "setItem">;
}) {
  const store = storage ?? (typeof window === "undefined" ? undefined : window.localStorage);
  const [mode, setModeState] = useState<ThemeMode>(() => readStoredTheme(store));
  const [prefersDark, setPrefersDark] = useState(() =>
    typeof window === "undefined" || typeof window.matchMedia !== "function"
      ? true
      : window.matchMedia("(prefers-color-scheme: dark)").matches,
  );

  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const query = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (event: MediaQueryListEvent) => setPrefersDark(event.matches);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);

  const resolved = resolveTheme(mode, prefersDark);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", resolved === "dark");
  }, [resolved]);

  const setMode = useCallback(
    (next: ThemeMode) => {
      setModeState(next);
      try {
        store?.setItem(THEME_STORAGE_KEY, next);
      } catch {
        // Storage is a convenience here, not a requirement.
      }
    },
    [store],
  );

  const value = useMemo<ThemeContextValue>(
    () => ({ mode, resolved, setMode, toggle: () => setMode(nextThemeMode(mode)) }),
    [mode, resolved, setMode],
  );

  return <ThemeContext value={value}>{children}</ThemeContext>;
}

export function useTheme(): ThemeContextValue {
  const value = useContext(ThemeContext);
  if (!value) throw new Error("useTheme must be used inside <ThemeProvider>");
  return value;
}
