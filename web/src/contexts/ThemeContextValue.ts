import { createContext } from "react";

export const THEMES = {
  DARK: "dark",
  LIGHT: "light",
} as const;

export type Theme = (typeof THEMES)[keyof typeof THEMES];

export interface ThemeContextValue {
  theme: Theme;
  toggleTheme: () => void;
}

export const ThemeContext = createContext<ThemeContextValue | null>(null);
