import { useEffect, useState, type PropsWithChildren } from "react";
import { ThemeContext, THEMES, type Theme } from "./ThemeContextValue";

const STORAGE_KEY = "loong-web-theme";

function getInitialTheme(): Theme {
  try {
    const saved = localStorage.getItem(STORAGE_KEY) as Theme | null;
    if (saved === THEMES.DARK || saved === THEMES.LIGHT) {
      return saved;
    }
  } catch {
    // Ignore storage failures.
  }

  if (typeof window !== "undefined") {
    return window.matchMedia("(prefers-color-scheme: light)").matches
      ? THEMES.LIGHT
      : THEMES.DARK;
  }

  return THEMES.DARK;
}

export function ThemeProvider({ children }: PropsWithChildren) {
  const [theme, setTheme] = useState<Theme>(getInitialTheme);

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    try {
      localStorage.setItem(STORAGE_KEY, theme);
    } catch {
      // Ignore storage failures.
    }
  }, [theme]);

  return (
    <ThemeContext.Provider
      value={{
        theme,
        toggleTheme: () => {
          setTheme((previous) =>
            previous === THEMES.DARK ? THEMES.LIGHT : THEMES.DARK,
          );
        },
      }}
    >
      {children}
    </ThemeContext.Provider>
  );
}
