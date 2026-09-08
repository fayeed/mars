import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";

import { nextThemeMode, readStoredTheme, resolveTheme, ThemeProvider, useTheme } from "./theme";

afterEach(() => document.documentElement.classList.remove("dark"));

function ThemeConsumer() {
  const { mode, resolved, toggle } = useTheme();
  return (
    <button type="button" onClick={toggle}>
      {mode}/{resolved}
    </button>
  );
}

describe("theme mode", () => {
  it("defaults to dark and rejects unknown stored values", () => {
    expect(readStoredTheme(undefined)).toBe("dark");
    expect(readStoredTheme({ getItem: () => "neon" })).toBe("dark");
    expect(readStoredTheme({ getItem: () => "light" })).toBe("light");
  });

  it("resolves system against the platform preference", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
    expect(resolveTheme("light", true)).toBe("light");
  });

  it("cycles dark, light, system", () => {
    expect(nextThemeMode("dark")).toBe("light");
    expect(nextThemeMode("light")).toBe("system");
    expect(nextThemeMode("system")).toBe("dark");
  });
});

describe("ThemeProvider", () => {
  it("stamps the resolved theme on the document and remembers the choice", async () => {
    const user = userEvent.setup();
    const memory = new Map<string, string>();
    const storage = {
      getItem: (key: string) => memory.get(key) ?? null,
      setItem: (key: string, value: string) => void memory.set(key, value),
    };

    render(
      <ThemeProvider storage={storage}>
        <ThemeConsumer />
      </ThemeProvider>,
    );

    expect(document.documentElement).toHaveClass("dark");

    await user.click(screen.getByRole("button", { name: "dark/dark" }));
    expect(document.documentElement).not.toHaveClass("dark");
    expect(screen.getByRole("button", { name: "light/light" })).toBeVisible();
    expect([...memory.values()]).toContain("light");
  });
});
