# frontend

The public **static site** for **fkst-hosted** — ChronoAI's hosted cloud for the fkst engine.
It is a marketing + docs site with two pages and **no backend, auth, or API calls**:

| Route          | Page             | Purpose |
|----------------|------------------|---------|
| `/`            | **Introduction** | What FKST is and what the hosted cloud service provides. |
| `/get-started` | **Get Started**  | Install the GitHub App, open a trigger issue, specify parameters, queue work, read status, and download logs. |

The Get Started content mirrors the operator manual at
[`../skills/fkst-control-plane-manual/SKILL.md`](../skills/fkst-control-plane-manual/SKILL.md);
keep the two in sync when the control-plane contract changes.

## Stack

- **React 18 + Vite + TypeScript**, routing via **react-router-dom**.
- **Tailwind CSS** with a dark, oklch design system (`src/styles/tokens.css`) —
  Space Grotesk (display), IBM Plex Sans (UI), IBM Plex Mono (mono), amber accent.
- **Vitest** + Testing Library (unit), **Playwright** (e2e smoke), **Storybook** (design docs).

## Develop

```bash
npm install        # first time
npm run dev        # http://localhost:3000
npm run typecheck  # tsc --noEmit
npm run lint       # eslint, zero warnings
npm run test       # vitest
npm run build      # static bundle to dist/
```

The production build in `dist/` is a plain static bundle — host it on any static file server or CDN.

## Deploy (GitHub Pages)

The site is published to **https://chronoaiproject.github.io/fkst-hosted/** by
`.github/workflows/deploy-pages.yml` on every push to `develop` that touches
`frontend/`. Because it's served under the `/fkst-hosted/` subpath, CI builds
with `VITE_BASE=/fkst-hosted/` (the Vite `base`), the router derives its
`basename` from `import.meta.env.BASE_URL`, and a `postbuild` step copies
`index.html` → `404.html` so deep links (e.g. `/get-started`) resolve on
refresh. To reproduce the deployed build locally:

```bash
VITE_BASE=/fkst-hosted/ npm run build && npm run preview
# → http://localhost:3000/fkst-hosted/
```

If a custom domain is added later, drop `VITE_BASE` (base becomes `/`).

## Layout

```
frontend/
├── src/
│   ├── app/            # App router + Shell (two-tab nav + footer)
│   ├── pages/          # introduction.tsx, get-started.tsx
│   ├── i18n/           # en/zh content catalog + LanguageProvider (see below)
│   ├── components/
│   │   ├── brand/      # FkstMark wordmark
│   │   ├── content/    # CodeBlock, Callout, Rich (inline markup)
│   │   └── layout/     # Eyebrow, SectionHeading, LanguageToggle
│   └── styles/         # design tokens + fonts
├── e2e/                # Playwright smoke test
└── mockups/            # legacy HTML mockups of the earlier app (reference only)
```

> The `docs/` and `mockups/` folders describe an earlier, backend-driven app design and are
> kept for reference only — they do not reflect the current static site.

## Internationalization

The site ships **English** and **简体中文** via an in-nav toggle. All copy lives in a typed
content catalog — `src/i18n/en.ts` and `src/i18n/zh.ts` (both implement `SiteContent` from
`types.ts`) — read through `useContent()` behind a `LanguageProvider`. The initial language is
detected from the browser and persisted to `localStorage`; `<html lang>` tracks the choice.

GitHub identifiers, code blocks, commands, and regexes are **not** translated — they live in
`src/i18n/literals.ts` so they can never drift between languages. Prose strings may use light
inline markup rendered by `<Rich>`: `` `code` `` → mono chip, `**bold**`, `*italic*`. To add a
locale, add a catalog implementing `SiteContent` and register it in `context.tsx`.
