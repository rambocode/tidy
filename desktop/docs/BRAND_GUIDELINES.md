# Tidy Brand and UI Guidelines

## Brand foundation

Tidy is the calm, reviewable desktop surface for Mole. It helps macOS power
users understand reclaimable space and approve maintenance with confidence.
The brand promise is **“See clearly. Tidy safely.”** This is a behavioral
standard, not marketing decoration: preview precedes execution, refusals name
their cause, and destructive actions remain reversible where supported.

Public product surfaces use **Tidy**. Rust crate names, IPC compatibility
events, shared operation logs, and references to the Mole CLI keep their
existing names. This preserves interoperability while giving the desktop app a
distinct identity.

Shared state remains under `~/.config/mole`; operation and deletion history
remain under `~/Library/Logs/mole`. The native bundle and LaunchAgent use
`com.tw93.tidy`; enabling or disabling login startup migrates the exact legacy
`com.cleaner.desktop` LaunchAgent created by placeholder development builds.

## Logo system

The Flow mark is a capital T formed by a graphite crossbar and turquoise folded
stem on a white rounded square. The curved seam represents a candidate moving
from inspection into an intentional action.

- Use `ui/public/tidy-app-icon.png` as the 1024 px master and
  `ui/public/tidy-logo.png` for compact UI placement.
- Keep clear space equal to one quarter of the mark's crossbar height.
- Minimum sizes: 24 px in UI, 32 px for standalone digital use.
- Place the complete tile on graphite, neutral, or photographic surfaces. Do
  not recolor, rotate, outline, add effects, or extract the T from its tile.
- The wordmark is plain `Tidy`, set in SF Pro Display Semibold. Do not place a
  tagline inside the app icon.

### Asset provenance

The Flow direction was selected by the project owner on 2026-08-17. The
production PNG master was generated from that approved concept with OpenAI
ImageGen; it contains no third-party source artwork. Until a hand-tuned vector
master is created, treat the 1024 px PNG as the geometry and color reference.

## Color

| Role | Token | Value | Use |
| --- | --- | --- | --- |
| Paper | `--brand-paper` | `#FFFFFF` | Logo tile, selected controls |
| Mist | `--brand-mist` | `#EAF8F7` | Light diagrams and documentation |
| Flow teal | `--brand-teal` | `#0CC2C7` | Brand mark and large accents |
| Bright teal | `--brand-teal-bright` | `#53D6D7` | Dark-mode links, focus, progress |
| Graphite | `--brand-graphite` | `#1E262F` | Primary shell and text on light |
| Raised graphite | `--brand-graphite-raised` | `#27323D` | Solid panels and controls |

Teal communicates selection and forward progress; it does not communicate
success. Use semantic green, amber, and red only for success, caution, and
failure. Never rely on color alone: pair status color with an icon and label.

## Typography and voice

Use SF Pro Display Semibold for page titles, SF Pro Text Regular/Medium for UI,
and SF Mono for paths, bytes, hashes, and logs. Chinese falls back to PingFang
SC. Use sentence case and tabular numerals for changing measurements.

| Style | Size / line height | Weight |
| --- | --- | --- |
| Display | 44 / 52 px | 700 |
| Page title | 22 / 28 px | 600 |
| Section title | 15 / 22 px | 600 |
| Body | 13 / 20 px | 400 |
| Caption | 12 / 16 px | 400 |

Write calm, literal interface copy. Name the action the user controls: “Review
12 items”, “Move to Trash”, or “Try scan again”. Confirmation and completion
must use the same verb. Errors state the cause and the next available action;
avoid vague messages and promotional language.

## Layout and spacing

Use a 4 px base unit through `--space-1` to `--space-6`. Default content width
is 980 px with 28 px window gutters. Dense rows are 44–52 px high; touch/click
targets are at least 32 px. Preserve one clear hierarchy per screen: title,
current measurement or task, primary action, then detail.

Use 8 px radii for compact controls, 12 px for panels, and 18 px for sheets.
Pills are reserved for the global navigation, filters, and small statuses. Do
not turn ordinary buttons and cards into pills.

## Component rules

### Navigation

The centered graphite pill is the stable app landmark. Show the 34 px Tidy
tile first, followed by short nouns: Clean, Apps, Optimize, Analyze, Status.
The active destination uses a white fill and graphite text. Navigation never
changes position between routes.

### Buttons and controls

- Primary: one per region; white fill on dark surfaces with graphite text.
- Secondary: transparent or raised graphite with a visible border.
- Destructive: red only at the final confirmed action, never for discovery or
  preview.
- Focus: 2 px bright-teal ring with 2 px offset. Keyboard focus is never
  removed without an equivalent replacement.
- Disabled controls retain their label and expose the reason in nearby copy or
  a tooltip; opacity alone is insufficient.

### Panels, lists, and data

Panels separate meaning, not decoration. Use one-pixel low-contrast borders and
avoid stacked shadows. Align file sizes and percentages by decimal position.
Paths use SF Mono, truncate in the middle when necessary, and reveal the full
value on hover or focus. Empty states state what was checked and offer the next
useful action.

### Preview and destructive flows

All destructive views follow the same sequence:

`Scan → Preview exact candidates → Confirm selection → Execute → Results`

The candidate count and bytes must remain stable across preview and confirm.
The final button names the outcome, such as “Move 12 items to Trash”. Results
separate completed, refused, and failed items; partial success is never rendered
as total success.

## Motion and accessibility

Use `140ms` for hover/focus feedback and `220ms` for sheets or route state
changes with `--ease-standard`. The signature motion is a single Flow sweep:
progress moves along a curved leading edge derived from the logo seam. Use it
only for scan-to-preview or execute progress. Respect `prefers-reduced-motion`
and replace movement with an immediate state change.

All body text targets WCAG AA contrast. Controls work by keyboard, icon-only
buttons have accessible names, and status updates use appropriate live regions.
Test at the minimum 800×560 window and with Chinese and English strings.

## Implementation map

- Product identity: `ui/src/brand.ts`
- Color, spacing, radius, and motion tokens: `ui/src/styles/tokens.css`
- Shared component styling: `ui/src/styles/base.css`
- Browser/navigation artwork: `ui/public/`
- Native app and tray artwork: `src-tauri/icons/icon.png`
- Native product name and bundle identifier: `src-tauri/tauri.conf.json`

When a token or interaction rule changes, update this document and the source
in the same change. New components should consume semantic tokens rather than
copying brand hex values into feature files.
