# fkst-hosted Design Language — "Elevated dark + amber"

Canonical source of truth for the frontend design system. Phase-2 agents
(motion, components, pages) MUST follow this file. It is a **premium refinement**
of the existing identity — amber-on-dark, Space Grotesk / IBM Plex Sans / IBM
Plex Mono — not a rebrand. Add depth, contrast, and energy; never break brand.

## Non-negotiable constraints

- **Behavior, DOM, roles, visible text, and all `data-tour` / `data-testid` /
  `aria-*` attributes are frozen.** Change visual styling + animation only.
- **Fixed-viewport layout is frozen.** The `h-[100dvh]` shell, the sole `<main>`
  scroller, and internal `ScrollArea` panels own all scrolling. Never introduce
  window/body scroll. The `--bg-glow` bloom is `background-attachment: fixed`
  paint on `<body>` — pure paint, no layout, no scrollbar.
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

### Surfaces (elevation ramp, dark blue-black, hue 255)

| Token | Tailwind | Purpose |
|---|---|---|
| `--bg` | `bg-bg` | App canvas (deepened; the `--bg-glow` bloom sits on top) |
| `--raise` | `bg-raise` | Panels, inputs, nav-active, card base |
| `--raise-2` | `bg-raise-2` | Hover/active raised, chips |
| `--line` | `border-line` (default border) | Hairline dividers |
| `--line-2` | `border-line-2` | Stronger border/outline |

### Text ramp (each nudged brighter for readability on the deeper bg)

| Token | Tailwind | Purpose |
|---|---|---|
| `--fg` | `text-fg` | Primary text |
| `--dim` | `text-dim` | Secondary text |
| `--faint` | `text-faint` | Tertiary text / labels |
| `--ghost` | `text-ghost` | Quiet meta text, eyebrows |

### Accent + semaphore

| Token | Tailwind | Purpose |
|---|---|---|
| `--amber` | `bg-amber` / `text-amber` | Brand accent |
| `--gold` | `bg-gold` / `text-gold` | Amber's **warmer** complement (hue 66); warning/stale + the `grad-accent` endpoint |
| `--green` | `bg-green` | Success semaphore |
| `--red` | `bg-red` | Danger semaphore |
| `--amber-ink` | `bg-amber-ink` | Dark ink for text on amber/gold fills |

### Gradients

| Token | Tailwind | Purpose |
|---|---|---|
| `--grad-accent` | `bg-grad-accent` | Brand fill: `linear-gradient(135deg, amber→gold)`. Primary buttons, `.grad-text` accents, `.grad-border` |
| `--grad-fg` | `bg-grad-fg` | Bright `fg→dim` vertical sweep for big display headings (low end stays legible) |
| `--grad-hairline` | `bg-grad-hairline` | Neutral top-lit hairline for `.grad-border` (light catching a raised edge) |
| `--grad-hairline-accent` | `bg-grad-hairline-accent` | Amber→gold diagonal hairline for accent/hero card borders |

### App bloom

| Token | Tailwind | Purpose |
|---|---|---|
| `--bg-glow` | `bg-bg-glow` | Faint radial bloom (warm amber top-center + cool blue undertone). Painted **fixed** on `<body>` behind all content. Do not re-apply elsewhere. |

### Glass

| Token | Tailwind | Purpose |
|---|---|---|
| `--glass` | `bg-glass` | Translucent `--raise` fill for backdrop-blur panels |
| `--glass-2` | `bg-glass-2` | Translucent `--raise-2` (nested/hover glass) |

Pair glass tokens with `backdrop-blur-glass` + a hairline border.

### Glows + shadows

| Token | Tailwind | Purpose |
|---|---|---|
| `--glow-amber` / `--glow-green` / `--glow-red` | `shadow-glow-amber` / `-green` / `-red` | Status-matched soft glow spreads |
| `--shadow-1` | `shadow-1` | Subtle depth (quiet raised elements) |
| `--shadow-2` | `shadow-2` | Card depth (default resting card) |
| `--shadow-3` | `shadow-3` | Raised/hover depth (hover-lift target) |
| `--shadow-glow` | `shadow-glow` | Card depth + amber bloom (primary surfaces) |
| `--highlight-top` | `shadow-highlight-top` | Inner 1px top highlight on raised cards |

Combine on hover, e.g. `hover:shadow-3` plus a glow.

---

## 2. Type scale (`tailwind.config.js` `fontSize`)

Fonts unchanged: **Space Grotesk** (`font-display`, headings), **IBM Plex Sans**
(`font-ui`, body — the default `font-sans`), **IBM Plex Mono** (`font-mono`,
code + eyebrows).

| Step | Tailwind | Spec | Use |
|---|---|---|---|
| Eyebrow | `text-eyebrow` | 11px / 0.18em | Mono uppercase labels (WINDOW, DEPLOYMENT) |
| Eyebrow-lg | `text-eyebrow-lg` | 12px / 0.16em | Larger section eyebrow |
| Body | `text-body` | 14px / 1.5 | Standard UI text |
| Nav | `text-nav` | 13.5px | Nav links |
| Modal title | `text-modal-title` | 19px / −0.01em | Modal headings |
| Display sm | `text-display-sm` | clamp 20→24px / 1.15 / −0.015em | Small section titles |
| Display md | `text-display-md` | clamp 24→32px / 1.1 / −0.02em | Panel/section headings |
| Display lg | `text-display-lg` | clamp 32→48px / 1.05 / −0.025em / 700 | Page headlines |
| Display xl | `text-display-xl` | clamp 40→64px / 1.02 / −0.03em / 700 | Hero headline |

Display sizes are `font-display`, tight tracking, weight 700 for lg/xl. Clamp
makes hero sizes fluid without media queries.

---

## 3. Utility-class vocabulary

Static utilities (`.grad-text`, `.grad-border`, `.glass`) and motion utilities
(`.anim-*`, `.hover-*`) are authored by phase-2 agents in `src/index.css`
(alongside the existing `.anim-row-in` etc.). Names below are canonical.

### Static (safe under reduced motion)

- **`.grad-text`** — clips `--grad-accent` (or `--grad-fg` via a `.grad-text-fg`
  variant) into text: `background: var(--grad-accent); background-size: 200% auto;
  -webkit-background-clip: text; background-clip: text; color: transparent;`.
  Use for the hero headline + accent words. The `200%` size lets `.anim-gradient-shift`
  shimmer it.
- **`.grad-border`** — gradient hairline border via a masked pseudo or a
  `padding: 1px; background: var(--grad-hairline)` wrapper (accent variant uses
  `--grad-hairline-accent`). One clean approach: element `background:
  linear-gradient(var(--raise),var(--raise)) padding-box, var(--grad-hairline)
  border-box; border: 1px solid transparent;`.
- **`.glass`** — `background: var(--glass); backdrop-filter: blur(12px);` +
  `-webkit-` prefix, typically with `.grad-border` and `shadow-2`. Prefer the
  `bg-glass backdrop-blur-glass` Tailwind utilities where inline is cleaner.

### Motion (all disabled/collapsed under reduced motion)

Registered keyframes live in `tailwind.config.js` (`gradient-shift`, `glow-pulse`,
`float`, `sheen`) → available as `animate-gradient-shift` etc. **Reference these
keyframe names; do not redefine them** in `index.css`. Existing keyframes
(`row-in`, `chip-in`, `notice-in`, `overlay-in`, `modal-in`, `drawer-in`,
`repo-pulse`, `node-glow`, `dot-blink`, `spin`, `shimmer`) stay in `index.css`.

| Class | Effect |
|---|---|
| `.anim-gradient-shift` | Slowly shimmers a 200% gradient (text/border). |
| `.anim-glow-pulse` | Breathing amber accent glow on active/primary elements. |
| `.anim-float` | Tiny vertical bob for hero accent marks. |
| `.anim-sheen` | One-shot highlight sweep across a surface (an overflow-hidden pseudo). |
| `.hover-lift` | On hover: `translateY(-2px)` + `shadow-3` + subtle glow, via **transition** (not keyframe), easing `emphasized`. Interactive cards only. |
| `.hover-underline` (underline-grow) | `scaleX(0)→1` underline on link/nav hover, `transform-origin:left`, transition. |
| existing: `.anim-row-in` / `.anim-chip-in` / `.anim-notice-in` / `.anim-overlay-in` / `.anim-modal-in` / `.anim-drawer-in` / `.anim-repo-pulse` / `.anim-node-glow` / `.anim-dot-blink` / `.anim-spin` / `.anim-shimmer` | Preserved as-is. |

**Choreography:** stagger list/section entrances with the existing `--stagger`
custom property on `.anim-row-in` (per-row delay). Keep entrance durations in the
150–320ms range with `emphasized` easing.

**Reduced-motion contract:** add every new `.anim-*` class to the
`@media (prefers-reduced-motion: reduce)` block in `index.css` with
`animation: none`. `.hover-lift` / `.hover-underline` set `transition: none` and
rest at their final state (no transform). Glass, gradients, shadows, static
glows, and the `--bg-glow` bloom remain.

---

## 4. Component recipes

- **Card** — `rounded-card` (or `panel`), `bg-raise` **or** `.glass`,
  `.grad-border` (or `border border-line`), `shadow-2`, optional
  `shadow-highlight-top`. Interactive cards add `.hover-lift` (→ `shadow-3` +
  glow on hover). Accent/hero cards use `--grad-hairline-accent` + `shadow-glow`.
- **Primary button** — `bg-grad-accent text-amber-ink`, `shadow-2` +
  `shadow-glow-amber`; hover raises brightness (`hover:brightness-110`) and glow.
  `rounded-control`.
- **Secondary button** — `.glass` or `bg-raise` + `.grad-border`;
  hover adds `shadow-glow-amber` (subtle) and `bg-raise-2`.
- **Chip** — `rounded-chip`, `bg-raise-2`, hairline; status chips carry a soft
  status-matched glow (`shadow-glow-green` / `-amber` / `-red`) and their existing
  text label (never color/motion alone).
- **Input** — `bg-raise` + `border-line`; focus shows an amber glow ring
  (keep the existing `:focus-visible` amber outline; add `focus:shadow-glow-amber`
  or a ring). Never remove the accessible focus outline.
- **Hero headline** — `text-display-xl font-display` + `.grad-text`, with a soft
  radial glow behind it and optional `.anim-gradient-shift`; a floating accent
  mark may use `.anim-float`.

---

## 5. Files & ownership

- `src/styles/tokens.css` — tokens + the `<body>` bloom rule (this phase). ✅
- `src/styles/fonts.css` — font `@import`s (unchanged pairing). ✅
- `tailwind.config.js` — theme mapping + registered motion keyframes. ✅
- `src/DESIGN-LANGUAGE.md` — this file. ✅
- `src/index.css` — motion agent authors the `.grad-text` / `.grad-border` /
  `.glass` / `.anim-*` / `.hover-*` utilities + the reduced-motion additions.
- Component/page agents apply the recipes above to `src/components` / `src/pages`
  with visual/animation changes only.
