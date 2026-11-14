// Aurora, Sage's colours, as `ui/src/index.css` declares them in
// `:root` (light) and `.dark`. index.css is the one the page uses; this table mirrors it so that
// a test can hold the two together and measure every pair of text on a background.

export type ThemeName = "light" | "dark";

export const TOKEN_NAMES = [
  "background",
  "foreground",
  "card",
  "card-foreground",
  "popover",
  "popover-foreground",
  "primary",
  "primary-foreground",
  "secondary",
  "secondary-foreground",
  "muted",
  "muted-foreground",
  "accent",
  "accent-foreground",
  "destructive",
  "border",
  "input",
  "ring",
  "sidebar",
  "sidebar-foreground",
  "sidebar-primary",
  "sidebar-primary-foreground",
  "sidebar-accent",
  "sidebar-accent-foreground",
  "sidebar-border",
  "sidebar-ring",
  "needs",
  "needs-foreground",
  "needs-surface",
  "needs-border",
  "bubble",
  "bubble-foreground",
  "success",
] as const;

export type TokenName = (typeof TOKEN_NAMES)[number];

/// Every token's value, `#rrggbb`. Where the spec's value missed AA it was darkened (light
/// theme only; the spec's section 9 lists each change): primary, bubble and success
/// `#0f8f7a` to `#0c7765`, needs `#b37800` to `#8f6000`, ring `#0f9f87` to `#0e9780`,
/// destructive `#c43d3d` to `#b23838`.
export const THEMES: Record<ThemeName, Record<TokenName, string>> = {
  light: {
    background: "#f6faf8",
    foreground: "#0b1512",
    card: "#ffffff",
    "card-foreground": "#0b1512",
    popover: "#ffffff",
    "popover-foreground": "#0b1512",
    primary: "#0a6f4d",
    "primary-foreground": "#ffffff",
    secondary: "#e1ebe6",
    "secondary-foreground": "#17302a",
    muted: "#e7f0eb",
    "muted-foreground": "#476056",
    accent: "#e1ebe6",
    "accent-foreground": "#17302a",
    destructive: "#b23838",
    border: "#cfdfd7",
    input: "#cfdfd7",
    ring: "#0b8a5f",
    sidebar: "#eaf3ee",
    "sidebar-foreground": "#0b1512",
    "sidebar-primary": "#0a6f4d",
    "sidebar-primary-foreground": "#ffffff",
    "sidebar-accent": "#d6ebde",
    "sidebar-accent-foreground": "#0b1512",
    "sidebar-border": "#cfdfd7",
    "sidebar-ring": "#0b8a5f",
    needs: "#8f6000",
    "needs-foreground": "#ffffff",
    "needs-surface": "#fff8e5",
    "needs-border": "#ecd28c",
    bubble: "#0a6f4d",
    "bubble-foreground": "#ffffff",
    success: "#0a6f4d",
  },
  dark: {
    background: "#080d0c",
    foreground: "#e8f1ec",
    card: "#0e1614",
    "card-foreground": "#e8f1ec",
    popover: "#0e1614",
    "popover-foreground": "#e8f1ec",
    primary: "#5fe0a6",
    "primary-foreground": "#03170f",
    secondary: "#16221f",
    "secondary-foreground": "#d3e6dd",
    muted: "#111c19",
    "muted-foreground": "#8aa79b",
    accent: "#16221f",
    "accent-foreground": "#d3e6dd",
    destructive: "#f78e8e",
    border: "#1c2b27",
    input: "#1c2b27",
    ring: "#5fe0a6",
    sidebar: "#0b1211",
    "sidebar-foreground": "#e8f1ec",
    "sidebar-primary": "#5fe0a6",
    "sidebar-primary-foreground": "#03170f",
    "sidebar-accent": "#12302a",
    "sidebar-accent-foreground": "#e8f1ec",
    "sidebar-border": "#1c2b27",
    "sidebar-ring": "#5fe0a6",
    needs: "#ffc247",
    "needs-foreground": "#231800",
    "needs-surface": "#1b1a0e",
    "needs-border": "#4a3d16",
    bubble: "#14463a",
    "bubble-foreground": "#e2fff3",
    success: "#5fe0a6",
  },
};

/// A background: a token, or a token laid at `alpha` over another (`bg-destructive/10`).
export type Surface = TokenName | { tint: TokenName; alpha: number; over: TokenName };

/// Text (or a focus ring) on a background that a component uses, and the contrast it needs:
/// 4.5 for text, 3 for the focus ring. `theme` limits a pair to one theme.
export type ContrastPair = {
  label: string;
  text: TokenName;
  on: Surface;
  min: number;
  theme?: ThemeName;
};

const SURFACES: TokenName[] = [
  "background",
  "card",
  "popover",
  "muted",
  "secondary",
  "sidebar",
  "sidebar-accent",
  "needs-surface",
];

const tint = (token: TokenName, alpha: number, over: TokenName): Surface => ({
  tint: token,
  alpha,
  over,
});

const surfaceName = (on: Surface) =>
  typeof on === "string" ? on : `${on.tint}/${on.alpha * 100} over ${on.over}`;

function pairs(text: TokenName, on: Surface[], min = 4.5, theme?: ThemeName): ContrastPair[] {
  return on.map((surface) => ({
    label: `${text} on ${surfaceName(surface)}`,
    text,
    on: surface,
    min,
    theme,
  }));
}

export const CONTRAST_PAIRS: ContrastPair[] = [
  ...pairs("foreground", SURFACES),
  ...pairs("muted-foreground", SURFACES),
  ...pairs("card-foreground", ["card"]),
  ...pairs("popover-foreground", ["popover"]),
  ...pairs("sidebar-foreground", ["sidebar", "sidebar-accent"]),
  ...pairs("sidebar-accent-foreground", ["sidebar-accent"]),
  ...pairs("secondary-foreground", ["secondary"]),
  ...pairs("accent-foreground", ["accent"]),
  ...pairs("primary-foreground", ["primary"]),
  ...pairs("sidebar-primary-foreground", ["sidebar-primary"]),
  ...pairs("needs-foreground", ["needs"]),
  ...pairs("bubble-foreground", ["bubble"]),
  // Links, the rail's active icon, a run's "Running".
  ...pairs("primary", ["background", "card", "muted", "sidebar", "sidebar-accent"]),
  // A tool step's ✓, a run's "Succeeded".
  ...pairs("success", ["background", "card", "muted"]),
  // A failure, a ✕, a destructive button (`bg-destructive/10`, `dark:bg-destructive/20`).
  ...pairs("destructive", ["background", "card", "muted"]),
  ...pairs(
    "destructive",
    [tint("destructive", 0.1, "background"), tint("destructive", 0.1, "card")],
    4.5,
    "light",
  ),
  // Hovered (`hover:bg-destructive/15`).
  ...pairs(
    "destructive",
    [tint("destructive", 0.15, "background"), tint("destructive", 0.15, "card")],
    4.5,
    "light",
  ),
  // Dark: at rest `/15`, hovered `/20`, on every surface a destructive button sits on.
  ...pairs(
    "destructive",
    [0.15, 0.2].flatMap((alpha) =>
      (["background", "card", "muted", "needs-surface"] as const).map((over) =>
        tint("destructive", alpha, over),
      ),
    ),
    4.5,
    "dark",
  ),
  // Inactive tabs.
  ...pairs("muted-foreground", ["muted"]),
  // Status marks and counts are not text: 3:1 against the column and the rail.
  ...(["muted-foreground", "needs", "primary", "destructive"] as const).flatMap((mark) =>
    pairs(mark, ["sidebar", "sidebar-accent", "background"], 3),
  ),
  // The approval card's title, a call sent for approval, a warning line.
  ...pairs("needs", ["background", "card", "muted", "sidebar", "needs-surface"]),
  // Text on the error banner and on a diff's added and removed lines.
  ...pairs("foreground", [
    tint("destructive", 0.1, "background"),
    tint("success", 0.1, "background"),
    tint("destructive", 0.1, "muted"),
    tint("success", 0.1, "muted"),
  ]),
  // The keyboard focus ring against whatever it surrounds.
  ...pairs("ring", SURFACES, 3),
];

function rgb(hex: string): [number, number, number] {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  if (!m) throw new Error(`not a #rrggbb colour: ${hex}`);
  return [parseInt(m[1]!, 16), parseInt(m[2]!, 16), parseInt(m[3]!, 16)];
}

const toHex = (channels: number[]) =>
  `#${channels.map((c) => Math.round(c).toString(16).padStart(2, "0")).join("")}`;

/// WCAG 2.2 relative luminance of a `#rrggbb` colour.
function luminance(hex: string): number {
  const [r, g, b] = rgb(hex).map((c) => {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
}

/// WCAG 2.2 contrast ratio of two `#rrggbb` colours, 1 to 21.
export function contrastRatio(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
}

/// `colour` laid at `alpha` over `over`, as the browser paints it.
export function composite(colour: string, alpha: number, over: string): string {
  const top = rgb(colour);
  const bottom = rgb(over);
  return toHex(top.map((c, i) => c * alpha + bottom[i]! * (1 - alpha)));
}

/// The text colour and the background colour of `pair` in `theme`.
export function pairColours(
  theme: Record<TokenName, string>,
  pair: ContrastPair,
): [string, string] {
  const background =
    typeof pair.on === "string"
      ? theme[pair.on]
      : composite(theme[pair.on.tint], pair.on.alpha, theme[pair.on.over]);
  return [theme[pair.text], background];
}
