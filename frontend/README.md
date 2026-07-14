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

## Deploy (Kubernetes, like every fkst deployable)

The frontend ships as a container image — nginx serving the built SPA with a
deep-link fallback — plus sample manifests under `k8s_sample/`:

```bash
docker build -t fkst-frontend:dev frontend/
kubectl apply -n <ns> -k frontend/k8s_sample
```

The default build targets the SAME-ORIGIN topology: one ingress fronts both
this SPA and the backend (`k8s_sample/ingress.yaml`), so the login/dashboard
XHRs never cross origins and no CORS setup exists. Only a cross-origin
backend needs `--build-arg VITE_FKST_API_BASE=https://api.example.com`
(VITE_ vars bake into the bundle at build time). `npm run dev` proxies
`/api` to a local backend on :8080.

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
