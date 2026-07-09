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

## Layout

```
frontend/
├── src/
│   ├── app/            # App router + Shell (two-tab nav + footer)
│   ├── pages/          # introduction.tsx, get-started.tsx
│   ├── components/
│   │   ├── brand/      # FkstMark wordmark
│   │   ├── content/    # CodeBlock, Callout
│   │   └── layout/     # Eyebrow, SectionHeading
│   └── styles/         # design tokens + fonts
├── e2e/                # Playwright smoke test
└── mockups/            # legacy HTML mockups of the earlier app (reference only)
```

> The `docs/` and `mockups/` folders describe an earlier, backend-driven app design and are
> kept for reference only — they do not reflect the current static site.
