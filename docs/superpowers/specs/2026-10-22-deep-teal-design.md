# Deep Teal: the look of Dots (chat-first UI, part C)

- Date: 2026-10-22
- Status: approved in conversation (directions picked in the visual companion), awaiting review of this document
- Part of: the chat-first UI. A (`2026-10-22-chat-home-design.md`) and B (`2026-10-22-needs-you-design.md`) are merged; this is C.

## 1. Goal

Dots gets an identity of its own, a light and a dark theme with a switch, a calmer conversation and consistent polish, without moving any screen's layout.

Chosen in the visual companion: the **Deep Teal** direction (slate-teal neutrals, a teal accent, amber-gold for "needs you"); the **document** conversation style (the person's messages in bubbles, the dot writing on the page, tool steps as chips); the **Comfortable** density (14px text, 12px corners).

Today: shadcn's stock neutral palette, dark only (`class="dark"` hard-coded in `ui/index.html`, the toaster fixed to dark), Geist, and a transcript that frames every reply the same.

## 2. Tokens (`ui/src/index.css`)

The shadcn variables in `:root` (light) and `.dark` are replaced; the `@theme inline` names stay, so every component keeps working. New semantic variables are added and exposed to Tailwind the same way.

| Variable | Light | Dark |
|---|---|---|
| `--background` | `#f7fbfb` | `#0c1416` |
| `--foreground` | `#11201f` | `#e2eeee` |
| `--card`, `--popover` | `#ffffff` | `#0f1a1c` |
| `--muted` | `#e9f2f1` | `#132325` |
| `--muted-foreground` | `#4f6867` | `#8fa9a9` |
| `--primary` (accent) | `#0f8f7a` | `#22c3a6` |
| `--primary-foreground` | `#ffffff` | `#03211c` |
| `--secondary`, `--accent` | `#e3eceb` | `#1a2a2c` |
| `--secondary-foreground`, `--accent-foreground` | `#1d3332` | `#d5e6e5` |
| `--border`, `--input` | `#d3e3e1` | `#1f3133` |
| `--ring` | `#0f9f87` | `#22c3a6` |
| `--destructive` | `#c43d3d` | `#f07171` |
| `--sidebar` (rail, column) | `#eef5f4` | `#0f1a1c` |
| `--sidebar-accent` (selected) | `#dcefec` | `#123033` |
| new `--needs` (needs you) | `#b37800` | `#ffc247` |
| new `--needs-foreground` | `#ffffff` | `#231800` |
| new `--needs-surface` (approval card) | `#fff8e5` | `#1b1a0e` |
| new `--needs-border` | `#ecd28c` | `#4a3d16` |
| new `--bubble` (the person's message) | `#0f8f7a` | `#1a4a48` |
| new `--bubble-foreground` | `#ffffff` | `#e2fffa` |
| new `--success` | `#0f8f7a` | `#22c3a6` |

Every pair of text on a background used by a component meets WCAG 2.2 AA (4.5:1 for text, 3:1 for large text and for the focus ring against its background); a value that fails in the build is darkened or lightened until it passes, and the change is recorded in section 9.

Type and shape: Geist stays. `body` 14px, line height 1.55; headings `h1` 18px semibold, `h2` 15px semibold; small text 12px; mono stays the system mono. `--radius` 0.75rem (12px), with the existing smaller and larger steps derived from it.

## 3. The theme switch

- **Choice**: System, Light or Dark; System by default. Kept in `localStorage` under `dots.theme` (a per-device display preference, not stored by the core: no command or contract change).
- **Applied before the first paint**: a short inline script in `ui/index.html` reads the choice and `prefers-color-scheme` and sets `class="dark"` (or not) and `color-scheme` on `<html>`, so no wrong-theme flash. The hard-coded `class="dark"` goes.
- **`ui/src/lib/theme.ts`**: `themeChoice()`, `setThemeChoice(choice)`, `resolvedTheme(choice, prefersDark)`, `applyTheme()`; with System, a `matchMedia('(prefers-color-scheme: dark)')` listener follows Windows live. Storage that cannot be read or written counts as System.
- **The window's title bar** follows: `getCurrentWindow().setTheme('light' | 'dark' | null)` (null for System), with the permission `core:window:allow-set-theme` added to `apps/dots-app/capabilities/default.json`. A failure is ignored (the page is themed anyway).
- **Toasts** follow the resolved theme (`components/ui/sonner.tsx`).
- **Settings** gains an "Appearance" section (`AppearanceSetting.tsx`): a three-way choice with the current one selected; a change applies at once.

## 4. Components

- **Rail** (`app/Layout.tsx`): 46px wide; the active link a tinted pill (`--sidebar-accent`) with the icon in `--primary`; counts as now, the Home and Inbox counts in `--needs` when something needs the person, else muted.
- **Dots column** (`DotsColumn.tsx`, `StatusMark.tsx`, `UnreadBadge.tsx`): rows 10px padding, 10px corners; the open row `--sidebar-accent`. Status marks: needs you `--needs`, running `--primary` with a gentle pulse (off under `prefers-reduced-motion`), attention `--destructive`, off a hollow ring in `--muted-foreground`, idle `--muted-foreground` at 40%. Unread badge `--needs` when the dot needs the person, else `--muted` on `--muted-foreground`.
- **Conversation, document style** (`components/Transcript.tsx`, shared with the run screen):
  - the person's message: a right-aligned bubble (`--bubble`), 16px corners with a small one at the bottom right;
  - the dot's text: on the page, under a meta line "● <dot> · <time>" in `--muted-foreground`;
  - tool steps of a run: one row of small rounded chips (tool name, a ✓ or ✕ at the end of each), which expands to today's details on click; a call sent for approval shows its chip in `--needs`;
  - notes and questions: a soft card (`--muted`), a question's state in its meta line;
  - a failure: soft red text with a left rule in `--destructive`, not a filled alert (still `role="alert"`).
- **Run card** (`screens/home/RunCard.tsx`): no box; a 2px left rule in `--border`, the title line and status, the summary in `--muted-foreground`; open (approval, question) as today.
- **Approval card** (`components/ApprovalCard.tsx`, `BrowserApprovalCard.tsx`): `--needs-surface` with a `--needs-border` border and the title in `--needs`; the primary action in `--primary`.
- **Composer** (`screens/home/Composer.tsx`): 12px corners, a 1px `--input` border, the focus ring `--ring`; Send the primary button.
- **Buttons, inputs, focus** (`components/ui/button.tsx`, `badge.tsx`, `input.tsx`, `textarea.tsx`): primary `--primary`; secondary and outline tinted neutrals; every focusable control shows a 2px `--ring` outline with a 2px offset on keyboard focus (`focus-visible`).
- **Other screens** (Inbox, Library, Settings, the editor, the run screen): `components/Page.tsx` sets one header style (18px title, 24px padding, 16px between sections); cards 12px corners with a 1px `--border`; empty states an icon (lucide) and one muted line. No layout change.

## 5. Collisions with parallel work

Most of the change is the variable values in `ui/src/index.css` (the parallel developer's file, replaced wholesale in `:root` and `.dark`, the `@theme` names kept). Class edits, small, in `components/ui/{button,badge,input,textarea,sonner}.tsx`, `components/{Transcript,ApprovalCard,BrowserApprovalCard,Page}.tsx`, `app/Layout.tsx`, the home's `DotsColumn`, `StatusMark`, `UnreadBadge`, `RunCard`, `Composer`. New files: `ui/src/lib/theme.ts`, `ui/src/components/AppearanceSetting.tsx`, and their tests. `SettingsScreen.tsx` gains one line (the section), knowing that the parallel developer's open PR #17 edits that file too: whichever merges second resolves one small conflict. `ui/index.html` (the script, the class) and `apps/dots-app/capabilities/default.json` (one permission). No core, command, contract or migration change.

## 6. Edge cases

- **Storage unavailable** (private mode, blocked): System, and the switch still works for the session.
- **The system theme changes while Dots runs**, with System chosen: the app follows at once; with Light or Dark chosen: nothing changes.
- **High-contrast or forced colours** (Windows contrast themes): the browser's forced colours win; focus rings and borders stay visible (`forced-color-adjust` left at its default).
- **Reduced motion**: the running pulse and any transition stop.
- **Code and diffs** (the run screen's diff view, code blocks in markdown): stay monospace on a `--muted` background in both themes; syntax colours, if any, keep AA on both.

## 7. Testing

- Unit (Vitest): `theme.ts` (choice read and written, storage failing, the resolved theme for each choice and system preference, the listener only with System); the inline script's logic (the same function, exported for test); `AppearanceSetting` (shows the choice, a change applies and is kept); the toaster follows the theme.
- Existing tests: updated only where they name a class or a text that this changes (e.g. "failed" outcomes keep their words; a test that looked for the alert box looks for the role, which stays).
- Real app (debug build, sandbox data): axe-core (WCAG 2.2 A/AA) with no violations in both themes on the home (with badges, an approval, a run card, a question), the Inbox, Library, Settings and the editor; no console errors; the switch changes the page and the title bar at once and survives a restart; screenshots of each screen in light and dark.

## 8. Out of scope

Layout changes, new screens, a logo or custom icons, illustrations, animations other than the running pulse, per-dot colours, a font change, themes beyond light and dark, syncing the theme across devices.

## 9. As built

The plan (`docs/superpowers/plans/2026-10-22-deep-teal.md`) built sections 1 to 7 with these changes:

- **Colours darkened for AA** (light theme only; every dark value passed): `--primary`, `--bubble` and `--success` `#0f8f7a` to `#0c7765` (white on it was 4.02:1); `--needs` `#b37800` to `#8f6000` (3.54:1 on `--needs-surface`, 3.75:1 under white); `--ring` `#0f9f87` to `#0e9780` (3.00:1 on `--sidebar`); `--destructive` `#c43d3d` to `#b23838` (4.28:1 on its own 10% tint, the destructive button). `ui/src/lib/tokens.ts` mirrors `index.css`; its test holds the two together and measures every pair of text (4.5:1) and the focus ring (3:1) on the surfaces the components use, tints included.
- **The variables the table leaves out** take their pair's value: `--card-foreground`, `--popover-foreground`, `--sidebar-foreground` and `--sidebar-accent-foreground` are `--foreground`; `--sidebar-primary(-foreground)` are `--primary(-foreground)`; `--sidebar-border` is `--border`; `--sidebar-ring` is `--ring`. The chart colours are kept.
- **Before the first paint** (section 3): not an inline script but `ui/public/theme-boot.js`, loaded by a classic `<script src>` in `index.html`'s `<head>`: the app's content security policy (`default-src 'self'`) runs only the app's own scripts. It still runs before the page is drawn; its test runs the file itself against `resolvedTheme` for every choice and preference.
- **Storage**: one that cannot be read gives System, or the choice made in this session; one that can be read but not written keeps the choice on screen until the next start.
- **The rail** (section 4): "something needs the person" is a pending approval or an open question (the rail does not load each dot's counts); both counts carry `data-tone` (`needs` or `muted`).
- **Tool steps** (section 4): the chips are the plain tool calls; check-ins, browser calls and unparsed lines keep their rows. A chip ends with ✓, ✕, … (running), – (no result) or ! (sent for approval, in `--needs`), and keeps the outcome's word for assistive technology; it opens to the call's one-line preview, its input and its result.
- **The meta line** shows the run's queued time; the run's header (trigger, time) stays above it. The run screen passes the dot's name too.
- **The composer** needed no edit of its own: its 12px corners, `--input` border, ring and primary Send come from `textarea.tsx`, `button.tsx` and the tokens.
- **Focus**: one base rule (`:focus-visible`, 2px `--ring`, 2px offset); the buttons, badges, inputs and text areas drop shadcn's blurred halo for it.
- **Type**: `body` 14px at 1.55 (Tailwind's `text-sm` line height too), `h1` 18px and `h2` 15px semibold in the base layer.
- **Reduced motion**: one rule stops every animation and transition; the running mark also has `motion-reduce:animate-none`.
- **Colours that assumed a dark page** now use the tokens: the run status badges, the diff view, the banners, the Library's status badges and warnings. Two lines of `SettingsScreen.tsx` (a tested endpoint's answer in `text-emerald-300`, an untested Claude version in `text-amber-300`) are left as they are while the parallel developer's PR #17 changes that file; they read poorly on the light theme until then.
- **Empty states**: the Inbox and the four Library tabs (an icon, one muted line); the home's first-dot screen keeps its call to action.
- **Settings** gained two lines: the import and the section.
- **Tests** are in `ui/src/lib/{tokens,theme}.test.ts(x)`, `ui/src/components/{AppearanceSetting,EmptyState}.test.tsx`, `ui/src/app/Look.test.tsx` and `ui/src/screens/home/Document.test.tsx`. The real-app checks of section 7 (axe-core in both themes, screenshots, the title bar) are run separately.

After the final review (2026-10-23): the contrast table now also measures the pairs the components use (destructive tints at rest and hovered on every surface, inactive tab text, status marks and counts as non-text at 3:1), and a test fails on any colour from Tailwind's palette in the UI and on a faint focus ring. To pass them: the dark `--destructive` is `#f78e8e` (was `#f07171`: on its 20% tint over `--muted` it measured 4.21); destructive buttons tint at rest `/10` light and `/15` dark, hovered `/15` and `/20`; switches, selects and tabs draw the shared 2px `--ring` outline; inactive tabs are `--muted-foreground`; the idle status mark is solid `--muted-foreground`; Settings' two status lines and the dot editor's three warnings use `text-success` and `text-needs`.
