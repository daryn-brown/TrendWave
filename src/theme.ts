import { useEffect, useState } from "react";

export type Theme = "light" | "dark";

// Kept in sync with the no-FOUC bootstrap script in index.html.
const STORAGE_KEY = "trendwave-theme";

export function getStoredTheme(): Theme | null {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    return v === "light" || v === "dark" ? v : null;
  } catch {
    return null;
  }
}

export function systemPrefersDark(): boolean {
  return (
    typeof window !== "undefined" &&
    !!window.matchMedia &&
    window.matchMedia("(prefers-color-scheme: dark)").matches
  );
}

/// Stored choice wins; otherwise fall back to the OS preference on first launch.
export function getInitialTheme(): Theme {
  return getStoredTheme() ?? (systemPrefersDark() ? "dark" : "light");
}

/// Reflect the theme on <html>: the `dark` class drives Tailwind's `dark:`
/// variant, and `color-scheme` themes native controls, scrollbars and the
/// window background.
export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  root.classList.toggle("dark", theme === "dark");
  root.style.colorScheme = theme;
}

export function storeTheme(theme: Theme): void {
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    /* persistence is best-effort */
  }
}

/// Theme state with a persisted, document-applied toggle.
export function useTheme(): { theme: Theme; toggle: () => void } {
  const [theme, setTheme] = useState<Theme>(getInitialTheme);

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  const toggle = () =>
    setTheme((current) => {
      const next: Theme = current === "dark" ? "light" : "dark";
      storeTheme(next);
      return next;
    });

  return { theme, toggle };
}
