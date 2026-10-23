// The Deep Teal colours (deep-teal spec section 2), as `ui/src/index.css` declares them in
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
    background: "#f7fbfb",
    foreground: "#11201f",
    card: "#ffffff",
    "card-foreground": "#11201f",
    popover: "#ffffff",
    "popover-foreground": "#11201f",
    primary: "#0c7765",
    "primary-foreground": "#ffffff",
    secondary: "#e3eceb",
    "secondary-foreground": "#1d3332",
    muted: "#e9f2f1",
    "muted-foreground": "#4f6867",
    accent: "#e3eceb",
    "accent-foreground": "#1d3332",
    destructive: "#b23838",
    border: "#d3e3e1",
    input: "#d3e3e1",
    ring: "#0e9780",
    sidebar: "#eef5f4",
    "sidebar-foreground": "#11201f",
    "sidebar-primary": "#0c7765",
    "sidebar-primary-foreground": "#ffffff",
    "sidebar-accent": "#dcefec",
    "sidebar-accent-foreground": "#11201f",
    "sidebar-border": "#d3e3e1",
    "sidebar-ring": "#0e9780",
    needs: "#8f6000",
    "needs-foreground": "#ffffff",
    "needs-surface": "#fff8e5",
    "needs-border": "#ecd28c",
    bubble: "#0c7765",
    "bubble-foreground": "#ffffff",
    success: "#0c7765",
  },
  dark: {
    background: "#0c1416",
    foreground: "#e2eeee",
    card: "#0f1a1c",
    "card-foreground": "#e2eeee",
    popover: "#0f1a1c",
    "popover-foreground": "#e2eeee",
    primary: "#22c3a6",
    "primary-foreground": "#03211c",
    secondary: "#1a2a2c",
    "secondary-foreground": "#d5e6e5",
    muted: "#132325",
    "muted-foreground": "#8fa9a9",
    accent: "#1a2a2c",
    "accent-foreground": "#d5e6e5",
    destructive: "#f07171",
    border: "#1f3133",
    input: "#1f3133",
    ring: "#22c3a6",
    sidebar: "#0f1a1c",
    "sidebar-foreground": "#e2eeee",
    "sidebar-primary": "#22c3a6",
    "sidebar-primary-foreground": "#03211c",
    "sidebar-accent": "#123033",
    "sidebar-accent-foreground": "#e2eeee",
    "sidebar-border": "#1f3133",
    "sidebar-ring": "#22c3a6",
    needs: "#ffc247",
    "needs-foreground": "#231800",
    "needs-surface": "#1b1a0e",
    "needs-border": "#4a3d16",
    bubble: "#1a4a48",
    "bubble-foreground": "#e2fffa",
    success: "#22c3a6",
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
  ...pairs(
    "destructive",
    [tint("destructive", 0.2, "background"), tint("destructive", 0.2, "card")],
    4.5,
    "dark",
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
