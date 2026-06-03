import { Monitor, Moon, Sun } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useTheme, type Theme } from "@/components/theme-provider";

const NEXT: Record<Theme, Theme> = {
  light: "dark",
  dark: "system",
  system: "light",
};

const ICON = { light: Sun, dark: Moon, system: Monitor } as const;

export function ModeToggle() {
  const { theme, setTheme } = useTheme();
  const Icon = ICON[theme];
  const next = NEXT[theme];

  return (
    <Button
      variant="ghost"
      size="icon"
      title={`Theme: ${theme} — switch to ${next}`}
      aria-label={`Current theme ${theme}. Switch to ${next}.`}
      onClick={() => setTheme(next)}
    >
      <Icon />
    </Button>
  );
}
