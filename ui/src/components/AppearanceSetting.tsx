import { cn } from "cn";
import { Monitor, Moon, Sun } from "lucide-react";
import { useState } from "react";

import { setThemeChoice, themeChoice, type ThemeChoice } from "@/lib/theme";

const CHOICES: { value: ThemeChoice; label: string; icon: typeof Sun }[] = [
  { value: "system", label: "System", icon: Monitor },
  { value: "light", label: "Light", icon: Sun },
  { value: "dark", label: "Dark", icon: Moon },
];

/// Settings' Appearance section (deep-teal spec section 3): System, Light or Dark, kept on this
/// device and applied at once. It is not part of Save.
export function AppearanceSetting() {
  const [choice, setChoice] = useState(themeChoice);
  function choose(next: ThemeChoice) {
    setThemeChoice(next);
    setChoice(next);
  }
  return (
    <fieldset className="flex max-w-3xl flex-col gap-3 rounded-lg border p-4">
      <legend className="px-1 text-sm font-semibold">Appearance</legend>
      <div className="flex w-fit gap-1 rounded-lg bg-muted p-1">
        {CHOICES.map(({ value, label, icon: Icon }) => (
          <label
            key={value}
            className={cn(
              "flex cursor-pointer items-center gap-1.5 rounded-md px-3 py-1 text-muted-foreground has-focus-visible:outline-2 has-focus-visible:outline-offset-2 has-focus-visible:outline-ring",
              choice === value && "bg-card text-foreground shadow-sm",
            )}
          >
            <input
              type="radio"
              name="theme"
              value={value}
              className="sr-only"
              checked={choice === value}
              onChange={() => choose(value)}
            />
            <Icon aria-hidden className="size-4" />
            {label}
          </label>
        ))}
      </div>
      <p className="text-xs text-muted-foreground">
        System follows Windows. Applies at once on this PC and is not part of Save.
      </p>
    </fieldset>
  );
}
