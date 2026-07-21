# fkst-hosted Design Language — "Electric dark" (v2)

Canonical source of truth for the frontend design system. This is the **v2
rebrand**: near-black neutral surfaces with a blue→violet accent gradient,
Space Grotesk / IBM Plex Sans / IBM Plex Mono, a single-viewport centered
landing hero, and a quieter glow vocabulary. Agents doing visual work MUST
follow this file.

**Legacy token names are load-bearing.** The accent pair keeps its historical
names — `--amber` holds the **blue** accent (oklch 66% 0.19 256) and `--gold`
the **violet** complement (oklch 62% 0.22 305) — because ~50 files consume the
`amber`/`gold` Tailwind utilities. Rebrands swap token **values**, never names.
Alternate accent sets from the design system (swap `--amber`/`--gold`/
`--amber-ink` together): Emerald `72% 0.15 165 / 78% 0.14 185 / 15% 0.03 165`;
Amber (the pre-v2 look) `82% 0.155 78 / 83% 0.14 66 / 22% 0.045 78`.

## Non-negotiable constraints

- **Styling passes change visuals only.** Behavior, DOM structure, roles, and
  all `data-tour` / `data-testid` / `aria-*` attributes are frozen during
  restyles. (Copy changes are product decisions, not styling — the v2 landing
  changed copy deliberately, through i18n, in both languages.)
- **Fixed-viewport layout is frozen.** The `h-[100dvh]` shell, the sole `<main>`
  scroller, and internal `ScrollArea` panels own all scrolling. Never introduce
  window/body scroll. The `--bg-glow` bloom is `background-attachment: fixed`
  paint on `<body>` — pure paint, no layout, no scrollbar. Full-height routes
  (dashboard, landing) pair with the pinned slim footer; scrolling doc routes
  keep the in-flow footer at end of content.
- **Every animation collapses to its instant final state under
  `prefers-reduced-motion: reduce`** (extend the existing `@media` block in
  `index.css`). Glass, gradients, shadows, and static glows MAY remain under
  reduced motion — only *motion* is disabled.
- **Every file < 500 lines.** oklch color convention. Comment the *why*.
- Tint recipe is always `color-mix(in oklab, var(--token) N%, transparent)` —
  never a second opaque color token.

---

## 1. Color tokens (`src/styles/tokens.css`)

All tokens are CSS custom properties on `:root`, mapped into Tailwind
(`tailwind.config.js`). Every **historical token name is preserved**.

### Surfaces (near-black neutral ramp, hue 260 at whisper chroma)

| Token | Tailwind | Value | Purpose |
|---|---|---|---|
| `--bg` | `bg-bg` | `oklch(8.5% 0.004 260)` | App canvas (the `--bg-glow` bloom sits on top) |
| `--raise` | `bg-raise` | `oklch(12.5% 0.005 260)` | Panels, inputs, nav-active, card base |
| `--raise-2` | `bg-raise-2` | `oklch(16.5% 0.006 260)` | Hover/active raised, chips, tracks |
| `--line` | `border-line` (default border) | `oklch(19.5% 0.006 260)` | Hairline dividers |
| `--line-2` | `border-line-2` | `oklch(27% 0.008 260)` | Stronger border/outline |

### Text ramp (unchanged from v1 — tuned for deep backgrounds)

| Token | Tailwind | Purpose |
|---|---|---|
| `--fg` | `text-fg` | Primary text |
| `--dim` | `text-dim` | Secondary text |
| `--faint` | `text-faint` | Tertiary text / labels |
| `--ghost` | `text-ghost` | Quiet meta text, eyebrows |

### Accent + semaphore

| Token | Tailwind | Value | Purpose |
|---|---|---|---|
| `--amber` | `bg-amber` / `text-amber` | `oklch(66% 0.19 256)` | Brand accent (**blue** in Electric) |
| `--gold` | `bg-gold` / `text-gold` | `oklch(62% 0.22 305)` | Accent complement (**violet**); `grad-accent` endpoint |
| `--green` | `bg-green` | `oklch(76% 0.14 158)` | Success semaphore |
| `--red` | `bg-red` | `oklch(68% 0.19 20)` | Danger semaphore |
| `--amber-ink` | `bg-amber-ink` | `oklch(99% 0.005 250)` | Near-white ink for text on accent-gradient fills |

### Gradients

| Token | Tailwind | Purpose |
|---|---|---|
| `--grad-accent` | `bg-grad-accent` | Brand fill: `linear-gradient(135deg, amber→gold)` = blue→violet. Primary buttons, `.grad-text` accents, `.grad-border` |
| `--grad-fg` | `bg-grad-fg` | Bright `fg→dim` vertical sweep for big display headings (low end stays legible) |
| `--grad-hairline` | `bg-grad-hairline` | Neutral top-lit hairline for `.grad-border` (light catching a raised edge) |
| `--grad-hairline-accent` | `bg-grad-hairline-accent` | Accent diagonal hairline for accent/hero card borders |

### App bloom

| Token | Tailwind | Purpose |
|---|---|---|
| `--bg-glow` | `bg-bg-glow` | The v2 dual radial wash: blue 9% from top-center + violet 6% at top-right. Painted **fixed** on `<body>` behind all content. Do not re-apply elsewhere. |

### Glass

| Token | Tailwind | Purpose |
|---|---|---|
| `--glass` | `bg-glass` | Translucent `--raise` (72%) for backdrop-blur panels |
| `--glass-2` | `bg-glass-2` | Translucent `--raise-2` (68%) — nested/hover glass |

Pair glass tokens with `backdrop-blur-glass` + a hairline border.

### Glows + shadows

v2 pulls glows in: `0 0 18px -4px` at a 32% mix — a quiet halo, not a bloom.

| Token | Tailwind | Purpose |
|---|---|---|
| `--glow-amber` / `--glow-green` / `--glow-red` | `shadow-glow-amber` / `-green` / `-red` | Status-matched soft glow spreads |
| `--shadow-1` | `shadow-1` | Subtle depth (quiet raised elements) |
| `--shadow-2` | `shadow-2` | Card depth (default resting card) |
| `--shadow-3` | `shadow-3` | Raised/hover depth (hover-lift target) |
| `--shadow-glow` | `shadow-glow` | Card depth + accent glow (primary surfaces) |
| `--highlight-top` | `shadow-highlight-top` | Inner 1px top highlight on raised cards |

`--ease` (`cubic-bezier(0.2,0.7,0.3,1)`) is the shared motion curve, mirrored
by Tailwind's `transition-emphasized`.

---

## 2. Type scale (`tailwind.config.js` `fontSize`)

Fonts unchanged: **Space Grotesk** (`font-display`, headings), **IBM Plex Sans**
(`font-ui`, body — the default `font-sans`), **IBM Plex Mono** (`font-mono`,
code + eyebrows).

| Step | Tailwind | Spec | Use |
|---|---|---|---|
| Eyebrow | `text-eyebrow` | 11px / 0.18em | Mono uppercase labels |
| Eyebrow-lg | `text-eyebrow-lg` | 12px / 0.16em | Larger section eyebrow |
| Body | `text-body` | 14px / 1.5 | Standard UI text |
| Nav | `text-nav` | 13.5px | Nav links |
| Modal title | `text-modal-title` | 19px / −0.01em | Modal headings |
| Display sm | `text-display-sm` | clamp 20→24px / 1.15 / −0.015em | Small section titles |
| Display md | `text-display-md` | clamp 24→32px / 1.1 / −0.02em | Panel/section headings |
| Display lg | `text-display-lg` | clamp 32→48px / 1.05 / −0.025em / 700 | Page headlines |
| Display xl | `text-display-xl` | clamp 40→64px / 1.02 / −0.03em / 700 | Large hero headline |

The v2 landing hero uses its own tighter metrics per the comp:
`clamp(36px,4.4vw,58px) / 1.04 / −0.045em / 700` (arbitrary classes on the h1).

---

## 3. Utility-class vocabulary

Static utilities (`.grad-text`, `.grad-border`, `.glass`) and motion utilities
(`.anim-*`, `.hover-*`) live in `src/index.css`. Names below are canonical.

### Static (safe under reduced motion)

- **`.grad-text`** — clips `--grad-accent` into text (accent words, the hero's
  second line). **`.grad-text-fg`** clips `--grad-fg` (display headings, the
  hero's first line). Both keep 200% background-size so `.anim-gradient-shift`
  can shimmer them.
- **`.grad-border`** — gradient hairline border: `background:
  linear-gradient(var(--raise),var(--raise)) padding-box, var(--grad-hairline)
  border-box; border: 1px solid transparent;` (accent variant uses
  `--grad-hairline-accent`).
- **`.glass`** — `background: var(--glass); backdrop-filter: blur(12px);` +
  `-webkit-` prefix. Prefer the `bg-glass backdrop-blur-glass` Tailwind
  utilities where inline is cleaner.

### Motion (all disabled/collapsed under reduced motion)

| Class | Effect |
|---|---|
| `.anim-gradient-shift` | Slowly shimmers a 200% gradient (text/border). |
| `.anim-glow-pulse` | Breathing accent glow on active/primary elements. |
| `.anim-float` | Tiny vertical bob for accent marks. |
| `.anim-sheen` | One-shot highlight sweep across a surface. |
| `.anim-beam-dot` | Traveling dot on the landing flow-line connectors (2.4s linear, second connector offset 1.2s). |
| `.hover-lift` | On hover: `translateY(-2px)` + `shadow-3` + subtle glow, via **transition**. Interactive cards only. |
| `.hover-underline` | `scaleX(0)→1` accent underline on link/nav hover. |
| existing: `.anim-row-in` / `.anim-chip-in` / `.anim-notice-in` / `.anim-overlay-in` / `.anim-modal-in` / `.anim-drawer-in` / `.anim-repo-pulse` / `.anim-node-glow` / `.anim-dot-blink` / `.anim-spin` / `.anim-shimmer` | Preserved as-is. |

**Choreography:** stagger list/section entrances with the `--stagger` custom
property on `.anim-row-in`. Keep entrance durations in the 150–320ms range with
`emphasized` easing.

**Reduced-motion contract:** add every new `.anim-*` class to the
`@media (prefers-reduced-motion: reduce)` block in `index.css` with
`animation: none`. `.hover-lift` / `.hover-underline` set `transition: none` and
rest at their final state. Glass, gradients, shadows, static glows, and the
`--bg-glow` bloom remain.

---

## 4. Component recipes

- **Card** — `rounded-card` (or `panel`), `bg-raise` **or** `.glass`,
  `.grad-border` (or `border border-line`), `shadow-2`, optional
  `shadow-highlight-top`. Interactive cards add `.hover-lift`. Accent/hero
  cards use `--grad-hairline-accent` + `shadow-glow`.
- **Primary button (app)** — `bg-grad-accent text-amber-ink`, `shadow-2` +
  `shadow-glow-amber`; hover `hover:brightness-110`. `rounded-control`.
- **Hero primary CTA (landing)** — the v2 white pill: `bg-fg text-bg
  rounded-pill px-[26px] py-[11px] font-semibold text-[13.5px]`, hover
  `opacity-85`. Reserved for the landing hero.
- **Secondary button** — `.glass` or `bg-raise` + `.grad-border`; hover adds
  `shadow-glow-amber` (subtle) and `bg-raise-2`.
- **Chip** — `rounded-chip`, `bg-raise-2`, hairline; status chips carry a soft
  status-matched glow and their text label (never color/motion alone).
- **Input** — `bg-raise` + `border-line`; focus shows an accent glow ring.
  Never remove the accessible focus outline.
- **Landing hero** — single-viewport centered column over an 80px masked line
  grid (opacity .4) + accent glow blob: mono eyebrow (10px/.2em uppercase
  ghost) → two-line headline (`.grad-text-fg` top, `.grad-text
  .anim-gradient-shift` bottom) → 15px dim lede (max 44ch) → CTA pair (white
  pill + quiet link) → the aria-hidden flow-line (`trigger issue ─●→ live
  session ─●→ a PR per task`, hidden under 560px viewport height).

---

## 5. Files & ownership

- `src/styles/tokens.css` — tokens + the `<body>` bloom rule.
- `src/styles/fonts.css` — font `@import`s (unchanged pairing).
- `tailwind.config.js` — theme mapping + registered motion keyframes.
- `src/DESIGN-LANGUAGE.md` — this file.
- `src/index.css` — the `.grad-*` / `.glass` / `.anim-*` / `.hover-*` utilities
  + the reduced-motion block.
- Component/page work applies the recipes above to `src/components` /
  `src/pages`.
