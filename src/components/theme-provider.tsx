import {
  createContext,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";

export type Theme = "system" | "light" | "dark";
type Resolved = "light" | "dark";

type ThemeContextValue = {
  theme: Theme;
  resolved: Resolved;
  setTheme: (theme: Theme) => void;
};

const ThemeProviderContext = createContext<ThemeContextValue | null>(null);
const STORAGE_KEY = "satchel-theme";

function systemPrefersDark(): boolean {
  return (
    typeof window !== "undefined" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches
  );
}

function resolve(theme: Theme): Resolved {
  return theme === "system" ? (systemPrefersDark() ? "dark" : "light") : theme;
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(
    () => (localStorage.getItem(STORAGE_KEY) as Theme | null) ?? "system",
  );
  const [resolved, setResolved] = useState<Resolved>(() => resolve(theme));

  useEffect(() => {
    const apply = () => {
      const next = resolve(theme);
      document.documentElement.classList.toggle("dark", next === "dark");
      setResolved(next);
    };
    apply();
    if (theme === "system") {
      const mq = window.matchMedia("(prefers-color-scheme: dark)");
      mq.addEventListener("change", apply);
      return () => mq.removeEventListener("change", apply);
    }
  }, [theme]);

  const setTheme = (next: Theme) => {
    localStorage.setItem(STORAGE_KEY, next);
    setThemeState(next);
  };

  return (
    <ThemeProviderContext.Provider value={{ theme, resolved, setTheme }}>
      {children}
    </ThemeProviderContext.Provider>
  );
}

// eslint-disable-next-line react-refresh/only-export-components
export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeProviderContext);
  if (!ctx) throw new Error("useTheme must be used within <ThemeProvider>");
  return ctx;
}
