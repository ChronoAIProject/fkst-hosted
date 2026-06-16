# fkst-hosted Frontend Architecture Brief

> **Status:** Authoritative for all frontend work in `fkst-hosted`. This document governs `frontend/`. It is grounded in the locked mockups, the locked design system, the Frontend TRD, the data reference, the **real** hosted backend v1 API (this repo, `develop`), and the repo conventions in the root `CLAUDE.md`. Where it conflicts with an aspirational doc, the **real v1 API and CLAUDE.md win**.
>
> **Source of the locked artifacts.** The locked design system is authored at `…/FKST/fe-blueprint/DESIGN.md`; the locked HTML mockups live at `…/.gstack/projects/FKST/designs/goal-board-20260611/`. The FE bootstrap **copies** these into the repo as `docs/design.md` (repo-root docs) and `frontend/mockups/` (reference artifacts checked in beside the app). They are inputs to build against, **not** a running app — there is no `frontend/package.json` / React app yet, and the FE CI workflow is intentionally a no-op until that file exists (§7).
>
> **The single most important rule:** the FE is a **read-mostly observer**. It must never become a second source of truth, never fabricate data, never show `0` for an unreachable source (show `unknown`), and never render a control for a capability that has no real backing — such gaps render as a **disabled control + an honest note**.
>
> **Revision (this version) — three cross-cutting concerns folded in:** **(a) cross-session persistence** of API reads so the console survives refresh and return visits (§2, §4 *Cross-session persistence*, §8) — a **last-known, as-of-stamped, stale-while-revalidate** cache, **never** presented as live and **never** a second source of truth; **(b) Storybook** as the living design-system surface (§2, §6, §9); **(c) mobile as a first-class target** (§6 *Responsive & mobile*, §9), honoring the locked DESIGN.md breakpoints (`1080·980·780·600·480`) for in-range layout, with any **sub-480** patterns/verification routed as amendments through **§11 decision 6**.

---

## 0. Implementation status — as-built (2026-06-16, PR #131)

> **For review.** This section records what is **actually implemented** on `feat/frontend-init` (PR #131) versus the design in the rest of this document, so the deltas are reviewable in one place. The design below is the **target**; this section is the **current truth**. Legend: ✅ conformant · 🟡 partial · ⚠️ deviation (built differently than designed) · ⏳ deferred (designed, not built).
>
> The first three layers landed via a dual-review loop (codex + Claude) + PM verification: **(1)** the v1-basic console shells, **(2)** the NyxID PKCE auth module, **(3)** the **API-integration epic** that wired the v1 endpoints + the v2 GitHub plane (gated) + the GitHub-account connect flow.

| Area | Design (this doc) | As-built | Status |
|---|---|---|---|
| Framework / Vite / TS-strict / Tailwind+oklch / Radix / React Router | §2 | As designed | ✅ |
| **Server state = TanStack Query** | §2, §43 | **All** data access is via TanStack hooks (`src/lib/hooks/*` use `useQuery`/`useMutation`/`useQueryClient`); screens consume hooks. Plain `fetch` exists **only** as the transport in `src/lib/api/client.ts` (`request<T>`/`requestVoid`) that the `queryFn` calls — **no raw fetch in screens/effects**. | ✅ |
| **Poll cadence (`refetchInterval`)** | §51 — ~5-min on GitHub/goals planes; fast on sessions; short on health | `useSessions` (2s while non-terminal, stops on terminal) ✅ and `useHealth` (30s) ✅. **`useGoals` / `useGitHubIssues` / `usePackages` have NO `refetchInterval` — they fetch once.** The ~5-min poll cadence on the goals/GitHub/packages planes is **not yet implemented**. | ⚠️ |
| **Cross-session persistence** (`persistQueryClient` + IndexedDB via `idb-keyval`) | §2, §4, §8 | **Implemented** — `PersistQueryClientProvider` + an idb-keyval async persister (`src/lib/persist/persister.ts`). Only **successful GET reads** are dehydrated (no mutations/errors/`unknown`), `maxAge` 24h, `buster`-versioned, `gcTime` ≥ maxAge, **wiped on sign-out**. Boot hydrates last-known reads, then revalidates. (`fake-indexeddb` polyfills the test env.) | ✅ |
| Forms (React Hook Form + Zod) | §2, §10 | New-goal modal + Add-package modal use RHF + Zod. | ✅ |
| Client/UI state (designed: Zustand) | §2 | UI-only state uses **React local state + a session-registry React context** (per-tab `Map<package, sessionId>`), **not Zustand**. No server data in any store. | 🟡 |
| **Hosted backend plane (v1)** | §3 Plane 2 (doc lists only health/packages/sessions) | **Fully wired, beyond the doc's table:** health; packages list/get/create + **update/delete/archive/generate/shares**; sessions create/get/stop; **and the entire Goals API — list/get/create/update/delete/trigger** (`src/lib/api/goals.ts`). Verified live against a local backend + Mongo. | ✅ |
| **GitHub plane (v2)** | §3 Plane 1 — "NyxID NOT integrated" | **Client + UI now built, gated on NyxID:** `github-issues` client + hooks, the **GitHub-account connect flow** (Settings `ConnectGitHub` CTA → `GET /github/accounts` list), and the accounts-gated **Issues view** + detail/comments. Degrades honestly when the proxy is absent (`503 "credential proxy not configured"`). No request fires without a connected account. So Plane 1 is **"built + gated," not "absent."** | 🟡 |
| NyxID auth | §3, §11 | PKCE module + provider + `/auth/callback` + bearer-on-client + env-gated login gate (`VITE_AUTH_REQUIRED`) are **built**; not yet pointed at a live NyxID deployment. | 🟡 |
| Honest gaps: lifecycle / merge-gate / redb / runs / consensus, Overview vitals / needs-you / stage pipeline, write posture | §4, §8 | Render as honest "not exposed by the v1 API" gaps — **no v1 endpoint exists**, so a loaded hosted goal never implies GitHub-marker/stage/gate/poll provenance. | ✅ |
| Storybook (living design-system) | §2, §6, §9 | **149 stories across 37 component groups** (primitives, layout, status, screens, the new ConnectGitHub/Issues/hosted states). `storybook-static` is **gitignored** — run `npm run storybook` (dev → :6006) or build+serve over HTTP; opening `index.html` via `file://` shows an empty shell. | ✅ |

**Open conformance gaps to review (designed but not built):**
1. **Poll cadence** — add `refetchInterval`/`staleTime` (~5 min) to `useGoals`/`useGitHubIssues`/`usePackages` so the goals/GitHub planes are poll-derived per §51 (sessions/health already are).
2. **UI store** — decide whether the session-registry context + local state is sufficient, or adopt Zustand as designed.
3. **`useArchiveReplace`** (PUT zip-replace of a package) is defined + tested but not yet surfaced in the UI — the only one of the 29 API operations without a UI consumer (create-from-zip `useArchiveCreate` is wired).

**Resolved since first audit:** cross-session persistence (§4/§8) is now implemented (`src/lib/persist/persister.ts`). The API back-check confirms **all 29 documented endpoints have a client fn + hook, and 28/29 are consumed in the UI** (the exception is `useArchiveReplace`, item 3).

These are intentional follow-ups, not silently skipped — each is a small, well-scoped change on top of the current TanStack layer.

---

## 1. Purpose, scope & v1 boundary

**What the FE is.** A single-page React **mission-control console** for developers running autonomous, GitHub-issue-driven AI dev loops over a hosted engine. It lives at `frontend/` in this monorepo, alongside the existing Rust/Axum `backend/`. It is a windowed, poll-derived **observatory** with a narrow set of real write actions.

**What it fronts (three planes — see §3).**
1. **GitHub plane** (goal-state truth) via NyxID's `api-github` proxy — *v1 gap, not integrated*.
2. **Hosted backend plane** (the Rust v1 API in this repo) — packages, sessions, health. *The only backend the FE calls directly.*
3. **Host-agent plane** (redb ledger/DLQ, logs, composed topology) — *optional, read-only, deferred*.

**The hard scope line (from `CLAUDE.md`).**

| In scope | Out of scope |
|----------|--------------|
| User-facing / public interfaces: Overview, Goals, Packages, Goal page, Settings, the Goal modal. | **The kernel engine.** The FE never changes, includes, or pressures engine internals. |
| Adapting to the existing v1 contract and rendering its gaps honestly. | Inventing backend endpoints, a query API, or a "dashboard as a second source of truth." |
| Reading `fkst-substrate` / `fkst-packages` behavior only to *understand contracts*. | Modifying the upstream engine/packages repos — they are **read-only references**. |

**No engine extension from the FE.** If a screen genuinely needs a read the backend doesn't serve (e.g. a posture-read endpoint, a config-read endpoint, a runs/journal-read endpoint), that work is **filed upstream as a backend issue** — the FE renders the gap honestly and does not fake it. The runtime vocabulary is exactly three levels — **Company (deployment/composed graph) · Department (graph node) · Person (one codex run)** — and the FE model never invents a fourth concept (no teams, sessions-as-agents, shared memory).

---

## 2. Tech stack & rationale

| Concern | Choice | Rationale | Deliberately **not** used |
|---|---|---|---|
| **Framework** | **React 18** SPA | Mandated by `CLAUDE.md` (Frontend: React). SPA, not SSR/RSC: there is no per-request server to render against; auth is a browser-side NyxID PKCE flow; data is client-polled. | Next.js/Remix (no SSR need; would add an app server the architecture doesn't have and complicate the NyxID-in-browser model). |
| **Build/dev** | **Vite** | Fast dev server + `import.meta.env` config, first-class TS, trivial static-asset output for the hosted-container serve story (§7). | CRA (deprecated), Webpack hand-config. |
| **Language** | **TypeScript** (`strict`) | The whole app is contract-shaped (12-state vocab, marker schemas, SessionView, PackageResponse). Types are the cheapest guard against "labels-as-state" / `unknown`-vs-`0` mistakes. | Plain JS. |
| **Styling + tokens** | **Tailwind CSS** themed from the **design system's oklch CSS custom properties**, with **shadcn/ui (Radix primitives)** for behavior-only components | The design system locks tokens as `oklch()` CSS custom properties → these become the Tailwind theme; shadcn gives accessible, unstyled-by-default primitives (dialog, dropdown, switch, tabs) we skin with our tokens. See §6. | Component libraries with baked-in visual identity (MUI, Chakra, AntD) — they fight the locked Linear/Vercel/Railway restraint and the single-amber-accent rule. No CSS-in-JS runtime. |
| **Routing** | **React Router** (data router) | Plain client routing for a flat IA (Overview/Goals/Packages/Goal page/Settings). The Goal **modal** is a routed-but-overlay pattern (open from anywhere; deep-link to the page). | File-based routing frameworks. |
| **Server state / data-fetching** | **TanStack Query (React Query)** | The entire app is **poll-derived server state**, not client state. Query gives us per-source caching, `refetchInterval` for the ~5-min cadence, `staleTime` tuning, retry/error surfaces, and request dedup. Each plane is a distinct query namespace with its **own freshness** (§4). | Redux/RTK for server data (wrong tool — this is cache-of-remote-truth, not app state); raw `fetch` in effects (loses dedup, freshness, retry). |
| **Persistence (cross-session)** | **TanStack Query persistence** (`persistQueryClient` + an **IndexedDB** persister via `idb-keyval`) | The console must survive **refresh and return visits**: on boot it **hydrates last-known reads** and paints instantly, then revalidates. Each persisted entry is **stamped with its `as-of`**, shown **stale-while-revalidate**, **`maxAge`-evicted**, **`buster`-versioned**, and **identity-scoped + cleared on sign-out** — a labeled, re-derivable snapshot, **never** live, **never** a second source of truth (§4, §8). | `localStorage` for large/sensitive GitHub data (size + leak risk → IndexedDB); persisting **mutations, errors, or `unknown` placeholders** (only successful idempotent GET reads are dehydrated). |
| **Client state** | React local state + a tiny store (Zustand) for **UI-only** state | Window selection, view toggle (Pipeline/Board, Issues/Activity), filters, condense-on-scroll, modal open. None of this is authoritative — it must never be persisted as truth. | A global store for server data (constraint #2: the FE stores only re-derivable, labeled cache). |
| **Forms** | **React Hook Form + Zod** | Only two real forms exist (New-goal modal, +Add package modal). Zod mirrors the backend's structure-only validation (name `^[A-Za-z0-9_-]+$`, ≥1 file, ≤256 files, size caps, engine-entry requirement) for instant client feedback, with the **server as final authority** (409/400 mapped inline, §10). | Heavy form frameworks. |
| **Testing** | **Vitest** (unit), **React Testing Library** (component), **Storybook test-runner (a11y + interaction)**, **Playwright** (e2e, incl. **mobile device projects**) | Reuse the existing backend e2e happy-path contract (`backend/tests`) as the FE e2e seam; Storybook is the component/a11y surface; Playwright covers phone + tablet viewports. See §9. | Enzyme; snapshot-only testing. |
| **Component workshop / design-system** | **Storybook** (+ addons: a11y/axe, viewport, interactions, controls) | The **single, runnable home of the locked design system**: every primitive/component in every state (loading/empty/error/unknown), token & status-vocabulary boards, a11y checks, and interaction tests. Built in CI; publishable as a static design-system site (§6, §9). | An ad-hoc **in-app component gallery route** (mixes design-system docs with product surfaces; loses a11y/viewport tooling). |
| **Lint/format** | **ESLint** (typescript-eslint, jsx-a11y, react-hooks) + **Prettier**, plus **repo-local anti-slop lint rules** (§6) | a11y and the design-system guardrails (no amber-as-status, status never hue-alone, no `0`-for-unknown) must be **enforced**, not just documented. | — |

**TanStack Query cadence policy (load-bearing).** The GitHub plane polls on a cadence **aligned to the ~5-min engine raiser cron** (`staleTime ≈ 5 min`, `refetchInterval ≈ 5 min` with on-demand refresh). This is the *observing* interval, **not** a transition cadence and **not** an engine constant. Session status (`GET /api/v1/sessions/:id`) polls **fast** while non-terminal (`pending|validating|running|stopping`) and **stops** on terminal (`stopped|failed`). Health (`GET /api/v1/health`) uses a short `staleTime`. **Nothing animates as "live."** On boot, the persisted cache (§4) **hydrates last-known reads** behind their `as-of` and then revalidates — first paint is **stale-while-revalidate, never shown as live.**

---

## 3. Access architecture — the three planes

```
                          ┌────────────────────────────────────────────────┐
  React SPA (browser)     │  holds ONLY a short-lived NyxID bearer token    │
  ─────────────────────── │  NEVER a raw GitHub token                       │
                          └────────────────────────────────────────────────┘
        │ (1) GitHub plane                │ (2) hosted backend       │ (3) host-agent
        ▼  v1 GAP                         ▼  PARTIAL (this repo)      ▼  DEFERRED
  Sign in with NyxID                GET  /health, /api/v1/health     redb ledger/DLQ
  (OAuth2 Auth-Code + PKCE)         GET  /api/v1/packages            framework/codex logs
        │                            GET  /api/v1/packages/:name      composed topology
        ▼                            POST /api/v1/packages (create)   (NOT covered by NyxID;
  NyxID api-github proxy            POST /api/v1/sessions             not reachable in v1)
  <NYXID_BASE>/api/v1/proxy/        GET  /api/v1/sessions/:id
    s/api-github/<github-path>      POST /api/v1/sessions/:id/stop
  Bearer <nyxid_token>
  (browser ↔ NyxID ↔ GitHub)
```

### Plane 1 — GitHub plane (TRUTH for goal state) · **v1 GAP**
- **Path:** every GitHub read/write goes through `<METHOD> <NYXID_BASE>/api/v1/proxy/s/api-github/<github-path>` with `Authorization: Bearer <nyxid_token>`. Path/method/body are identical to the GitHub REST API. **The browser never holds a raw GitHub token** — NyxID injects it server-side. The client **must not** send a second `Authorization` header or any GitHub token in body/query.
- **Owns:** issues/PRs, trusted `state:v1` comment markers (canonical goal state), `fkst-dev:*` labels (hints only), and the four legitimate GitHub mutations (open/comment/label/close/create-issue). Marker-trust and version-ordering run **client-side** (§4).
- **v1 reality:** **NyxID is NOT integrated.** No auth, no proxy wiring. Therefore every GitHub-plane surface (goal list/board, lifecycle markers, fire/pause label actions, avatar identity, connected repos) renders as **"Sign in with NyxID" disabled + honest note**, or an empty/loading state — **never fabricated issues**. *(As-built update — see §0: the NyxID PKCE module, the v2 `github-issues` client/hooks, the GitHub-account **connect flow**, and the accounts-gated **Issues view** are now **built and gated**; this paragraph describes the original v1 cut, and the surfaces still degrade honestly until a live NyxID proxy is wired.)*
- **Migration path:** land NyxID (`@nyxids/oauth-react` PKCE login → token → proxy calls). Confirm the three open NyxID items in §11 first.

### Plane 2 — Hosted backend plane (this repo's Rust v1 API) · **PARTIAL, the only directly-called backend**
Confirmed from `backend/.../router.rs`: routes are exactly `health`, `/api/v1/health`, and the `/api/v1` nest of `packages` + `sessions`. **CORS is `allow_origin(Any)`** (a dev-only TODO in the source).

| Endpoint | What the FE does with it |
|---|---|
| `GET /health`, `GET /api/v1/health` → `{status, mongo, version}` | `useHealth()` short-`staleTime`; drives a degraded banner off **HTTP 200 vs 503** (Mongo-down returns 503 with body). `version` is the **backend crate** build version — labeled as such, **not** the product version. **Does NOT report GitHub sync freshness.** |
| `GET /api/v1/packages` → `string[]` | `usePackagesList()`; render `[]` as a real empty state. Names only. |
| `GET /api/v1/packages/:name` → `{name, files[], composed_deps[], created_at, updated_at}` | `usePackage(name)`; drives flat/composed badge (`composed_deps.length>0`), deps chips, read/write tri-panel from `files[].path`. `updated_at` is set on update. |
| `POST /api/v1/packages` | `useCreatePackage()`; 201 → invalidate list; **409 → inline "name already exists (a revision is a new name)"**; 400 → field-level validation. |
| `PUT /api/v1/packages/:name` | (Update/replace package) - Supported by backend, UI coming soon |
| `DELETE /api/v1/packages/:name` | (Delete package) - Supported by backend, UI coming soon |
| `POST /api/v1/sessions` `{package_name}` | `useCreateSession()`; 201 `{id, status:"pending"}` → begin polling. **409 → "this package already has a live session — stop it first"** (one live per package, lease-fenced; the lease `_id` IS the package name — confirmed in `models.rs::LeaseDoc`). |
| `GET /api/v1/sessions/:id` → `SessionView` | `useSession(id)` with `refetchInterval` while non-terminal; the **source of truth for run/session status**. On `failed`, render `error` (e.g. conformance failure). `run_key` is **not** in `SessionView`. Don't surface `pod_id/fencing_token/pid/runtime_dir` as user content. |
| `POST /api/v1/sessions/:id/stop` → `202 {status:"stopping"}` | `useStopSession()`; the 202 is only an ack — **keep polling GET until `stopped`/`failed`.** Stops **one package's session** (the lease is per package), never "the deployment." |

**"Apply changes / supervise restart" is client-orchestrated** (no single endpoint): `POST stop` → poll `GET` until `stopped` → `POST /sessions {package_name}`. Show progress across the three calls.

### Plane 3 — Host-agent plane (engine-host local data) · **DEFERRED**
redb delivery ledger/DLQ, framework-child/codex logs, runtime composed topology, codex-permit concurrency. **NyxID does not cover this** and the hosted v1 API does not serve it. The engine is **hosted, not local**, so "connect a local host agent" is **forbidden** in v1. Every host-agent-sourced value (Activity runs table, redb delivery panels, in-DLQ count) renders **disabled / "unknown" with an honest "host telemetry not connected" note** — never `0`, never fabricated rows.

---

## 4. Data & truth model

### Source-of-truth hierarchy (encode this exactly)
1. **TRUTH tier** — redb delivery ledger/DLQ **+** GitHub/git (issues, PRs, commits, the durable run journal).
   - **Goal state + lifecycle** authority = trusted-bot `state:v1` (and sibling `fkst:github-devloop:*`) markers authored by `FKST_GITHUB_BOT_LOGIN`, ordered by the version tuple.
   - **Run/session state** authority = the hosted backend's session doc via `GET /api/v1/sessions/:id` (Mongo-backed, CAS-guarded).
2. **HINT tier** — `fkst-dev:*` labels: fast board coloring only, **subordinate to markers** when they disagree.
3. **DIAGNOSTICS tier** — framework-child/codex/department logs: never authoritative; explain *why* only.

**The hosted session doc is authority for run lifecycle but NOT for goal lifecycle (GitHub is).** Never write a derived value back as if it were a source.

### Marker trust + version ordering (mandatory, client-side)
- **Trust:** honor **only** markers authored by `FKST_GITHUB_BOT_LOGIN`. Neutralize untrusted `<!-- fkst:` text client-side (`&lt;!--`). `fkst-dev:*` labels are hints. *(v1: `FKST_GITHUB_BOT_LOGIN` has no read endpoint — render trust as a generic "trusted-bot" chip until it can be read.)*
- **Version ordering:** current state = the **max** marker under the tuple `(updated_at, loop_n, fix_n, review_meta_action_n, review_loop_n, stage_rank)`, tie-break **blocked > ready**. "Most recent comment" is **not** the current state.
- **12-state vocab + terminality:** `thinking, ready, implementing, pr-open, reviewing, merge-ready, merging, fixing, review-meta` are non-terminal; **`impl-failed, blocked, merged` are TERMINAL** — **no console out-edges**. Re-engaging a terminal goal is a **new GitHub fact** (new issue), never reopen/resume/retry.

### Freshness model — per-source `as-of`, never one global "now"
- **Data is POLL-derived (~5-min cron ticks), not live — and the FE says so.** Use "polled / last observed / as of," never "live."
- **The GitHub freshness chip is FE-tracked from the last successful proxy fetch — NOT `GET /api/v1/health`.** Conflating backend health with GitHub sync would be a dishonest green.
- Show a **per-source freshness strip**; on fused cards show the **deciding source's** freshness (state ← GitHub, delivery ← redb). Stale: **warn after 1 missed poll, critical after 2.**

### `unknown`, never `0` (load-bearing)
An unreachable source/service reads the literal token **`unknown`**. `"no data" ≠ "not connected"`; `"healthy" ≠ "unreachable"`; `"0 dead-letters" ≠ "host agent offline"`. A `0` is truthful **only** when the poll genuinely succeeded and returned none. **Posture (`FKST_GITHUB_WRITE`) has no read endpoint in v1 → posture is `unknown`; the FE must never assert "REAL."**

### State taxonomy every data surface implements
| State | Rule |
|---|---|
| **loading** | Skeleton rows/cells; freshness chip shows neutral `syncing…` (not green). The first poll is *loading*, **never animated as live**. Counts read `—`, never `0` or a fabricated number. |
| **empty** | A *successful* empty poll: real `0`s + a quiet empty row ("Nothing needs you" / "No goals in this window"). The conduit/architecture chrome still renders. Distinct from unreachable. |
| **error** | Per-source error; freshness dot non-green with "last synced `<as-of>` (stale)." Freeze last-known values; **never blank the whole screen** because one source failed. |
| **unknown** | Any deciding source unreachable → literal `unknown` + as-of chip. Posture-dependent UI ("REAL", "WRITE: REAL") renders `posture unknown (deploy-time)`. |
| **hydrated (stale)** | On boot, a **persisted last-known** value (below) paints behind its `as-of` chip marked **"last synced `<as-of>` · revalidating…"** — *not* green, *not* "live." **Staleness is judged at hydrate:** an entry already past the critical threshold, or whose deciding source fails its first revalidation, renders **`unknown`** from frame one — never a confident stale number — and is replaced when a fresh poll returns. |

### Cross-session persistence — last-known, as-of-stamped (survives refresh & return visits)
The console **persists successful API reads** so a refresh or a later visit paints **last-known state instantly**, then revalidates — without ever becoming a second source of truth. Mechanism: TanStack Query's `persistQueryClient` over an **IndexedDB** persister (`idb-keyval`).

**Non-negotiable rules (where persistence meets the honesty doctrine):**
1. **As-of travels with the data.** Every persisted entry carries the `as-of` of the poll that produced it; on hydration the UI shows **"last synced `<as-of>` · revalidating…"** — never "live," never green.
2. **Stale-while-revalidate, evaluated at hydrate time.** Hydrated data is a *first paint*, immediately reconciled against a live poll. **Staleness is judged the moment of hydration, not only after a failed revalidation:** any entry already past the **critical staleness threshold** paints as **`unknown` / "stale, revalidating"** from frame one — never a confident number — even before (or if never) the revalidation poll resolves (offline). Old numbers are never re-presented as current.
3. **Reachability gates the degrade, not just age.** A hydrated value whose **deciding source fails its first revalidation attempt** (source currently unreachable) renders **`unknown` immediately**, independent of `maxAge`/age — the `unknown`-never-`0` rule, applied to cached data. (Host-agent/DLQ counts are not persisted in v1; this rule holds when they land.)
4. **`maxAge` ≤ critical cutoff, + `buster`.** `maxAge` is pinned **at or below the critical-staleness cutoff**, so an entry can never outlive its honest-render window — an "older-than-critical but younger-than-`maxAge`" confident stale number is impossible by construction. A `buster` keyed to the **app/schema version** invalidates the whole store on a shape change — no resurrecting incompatible data. (Exact values pinned in M2, §11.)
5. **Dehydrate only re-derivable GET reads — and revalidate terminal sessions on hydrate.** Persist **only successful, idempotent reads** (health, packages, GitHub goal/marker reads, and sessions). **Never persist** mutations, in-flight queries, errors, or the literal `unknown` placeholder. **Sessions need special care:** session polling *stops* on a terminal state (`stopped`/`failed`), so a hydrated terminal `SessionView` has no steady-state poll to reconcile it — it **must fire a one-shot `GET /api/v1/sessions/:id` on mount**; a `404`/error or a past-critical age degrades it to **`unknown`**, never a confident terminal badge. A package's *current* run is reconciled via its live session (`POST /sessions` → `409` identifies the live one), so a superseded session id is never shown as the package's current run. The backend session doc stays authority for run lifecycle (hierarchy above); the cache never overrides it.
6. **Whole-store wipe on sign-out / identity change.** The store is **namespaced by the authenticated NyxID subject**, and **sign-out or an identity change drops the ENTIRE IndexedDB store — all planes, not just the GitHub partition.** Backend-plane reads (packages/sessions) are per-user-initiated and can carry run state, so they are wiped too; nothing from user A hydrates into user B's session on a shared machine (§8).
7. **Truth tiers unchanged; persisted markers are re-filtered, not re-authenticated.** Persistence is a render-latency optimization on the observed/HINT layer. Marker-trust and version-ordering **re-run on every hydrate and every poll** using the current client-side trust config — but because `FKST_GITHUB_BOT_LOGIN` is unreadable in v1 (above), persisted markers are **re-filtered, not re-authenticated**: treat them as **hint-tier on hydrate until a live proxy fetch re-confirms.** A persisted marker set is never trusted *because* it was cached.

---

## 5. Information architecture & routing

**Primary nav: Overview · Goals · Packages.** **Settings opens from the AVATAR**, not nav. **Inbox is deferred** — spec-retained but **not in nav and not a registered route** in v1 (it ships later as the Overview "Needs you" surface). The **Goal detail** is reachable as both an **Issue MODAL from anywhere** and a **full goal PAGE**. **Runs is folded into Goals** as the Issues/Activity toggle — there is no dedicated Runs route.

| Route | Screen | Primary data source(s) | v1 status |
|---|---|---|---|
| `/` → `/overview` | Overview (Pipeline + Board, vitals, Needs-you) | GitHub plane (derived goal set, vitals) + backend `/health` for the degraded note | **Mostly gap** (GitHub plane behind NyxID); honest skeleton/empty in v1 |
| `/overview` | same | — | — |
| `/goals` | Goals — **Issues** view (hero table) | GitHub plane (issues + markers) | **Gap** (NyxID) |
| `/goals?view=activity` | Goals — **Activity** view (folded Runs; reached only via the Goals Issues/Activity toggle) | **Host-agent plane** (logs/redb) | **Deferred** — "host telemetry not connected" |
| `/goals/:id` | Goal **page** (decision header, timeline, diagnostics) | GitHub plane (markers/PR) + host-agent (redb/codex panels) | **Gap + deferred**; only the New-goal modal's package graph is backend-served |
| Goal **modal** (overlay from any goal) | same data as goal page | GitHub plane | **Gap** |
| `/packages` | Packages (list, topology, read/write tri-panel) | **Backend** `GET /packages`, `/packages/:name`; topology = FE-derived from `files[]` | **Available** (list/detail/create); topology/conformance/posture/source-link = gap |
| `/settings` (from avatar) | Settings & Safety | Backend `/health`, `GET/POST/stop /sessions`; NyxID identity/repos/posture | **Mixed** — engine pane + per-package stop available; identity/posture/knobs gap |

> **No `/runs` route.** DESIGN.md folds Runs into Goals; minting even a redirect would re-establish the retired Runs identity. Activity is reached only through the Goals Issues/Activity toggle (`/goals?view=activity`). **No `/inbox` route either** — Inbox is unbuilt and spec-retained; if a placeholder is ever wanted, it redirects to Overview "Needs you" with a plain "unbuilt" note rather than presenting a deep-linkable screen.

**Backend-served, truthful-in-v1 surfaces** are exactly: the **Packages** list/detail/create + session cycle; the **New-goal modal's read-only package graph** (`/packages` + `/packages/:name` → `composed_deps`); the **Settings hosted-engine connection pane** (`/health` + session status); and a **per-package "Stop session for `<package>`"** (`POST /sessions/:id/stop` → poll). There is **no deployment-wide suspend** — leases are per package (`LeaseDoc._id == package_name`), so a single stop ends one package's session, not the deployment; any "Suspend deployment" affordance is a **v1 GAP** (disabled + note: *"v1 sessions are per-package; no deployment-wide suspend endpoint"*). **Everything goal/marker/PR/redb is GitHub-plane + host-agent and renders honestly-disabled/unknown in v1.**

---

## 6. Design-system implementation

The design system (authored at `fe-blueprint/DESIGN.md`, copied into the repo as `docs/design.md`) is **locked**. It becomes code in five layers (1–5), with cross-cutting responsive, accessibility, and anti-slop rules below them:

**1. Token layer (oklch CSS custom properties).** Tokens are authored as `oklch()` CSS custom properties on `:root` (and a `[data-theme]` if ever needed). This is the single source; nothing hardcodes a hex except the **red semaphore** (`--red: oklch(67% 0.18 18)`) and the documented green/gold semaphores where the design system pins them.

**2. Tailwind theme mapping.** `tailwind.config` maps theme colors/spacing/radii/typography to the CSS variables (`colors.bg: 'var(--bg)'`, etc.) so utilities resolve to tokens. Fonts: **Space Grotesk** (display), **IBM Plex Sans** (UI — explicitly **not** `system-ui`), **IBM Plex Mono** (ids/sha/ts, `tabular-nums`). One **amber** accent registered as **brand only**.

**3. shadcn/Radix component set (behavior, our skin).** Dialog (Issue modal, New-goal, +Add package), Dropdown/Select (scope, filters), Switch (enable toggle, posture — disabled in v1), Tabs/segmented (window, view-switch, Issues/Activity, status filter), Tooltip (freshness chip), Toast (sparingly). All restyled to tokens; **no library default visual identity ships.**

**4. Layout + chrome.**
- **Condense-on-scroll topbar** with **hysteresis 140/40** (condense >140px, expand <40px), revealing `.minis` current-setting chips when condensed.
- **Max width 1440px, centered.**
- **No plain `1fr` grids:** every responsive grid uses `minmax(0, 1fr)` **with `min-width: 0`** on children. **Verify `scrollWidth ≤ clientWidth`** in tests (§9) so nothing silently overflows the 1440 cap.

**5. Storybook — the living design-system surface.** Storybook is the canonical, runnable home of the **locked** DESIGN.md and the **honesty states** — not a nice-to-have. A component is **not "done"** until the *necessary* requirements below pass. (Required = blocks merge; Should = expected; Not required = out of v1 scope.)

***Required — every reusable component in `src/components/` ships a co-located `Component.stories.tsx` covering:***
- **The locked component vocabulary (DESIGN.md §7):** topbar (+ condense-on-scroll), nav, window segmented control, vitals panel/cell, pipeline stage, board card, issue-modal shell, the list/row patterns (`.plist` / `.levels` / `.rw`), package row, topology adjacency row, posture chip, GitHub freshness chip, state dot, 12-state badge, CI glyph, primary/secondary/danger buttons, toggle/switch, and the +New goal / +Add package modal bodies. (One reusable component → one stories file.)
- **The data-state matrix** — for any data-bearing component, **one story each** for **`loading` · `empty` · `error` · `unknown`** (the §4 taxonomy), a **`hydrated (stale)`** story, and a **disabled-gap** variant wherever the component fronts a v1 gap (§10). *This is why Storybook is necessary:* it is where **`unknown`-not-`0`**, **stale-not-live**, and **disabled-control-+-note** are demonstrated and regression-tested, not just coded.
- **The interaction-state matrix** (DESIGN.md §7.1): `default` / `hover` / `active` / **`focus-visible` (amber ring)** / `disabled`, plus a **`prefers-reduced-motion`** variant.
- **Responsive proof** — rendered via the **viewport** addon at the locked breakpoints (1080·980·780·600·480) + the 360 phone test floor.

***Required — foundation / reference stories (the design system made tangible):***
- **Token boards** rendered straight from the CSS custom properties: color, typography (the three fonts), spacing, radii — a token change is visible, not buried.
- **Status-vocabulary board** proving every status reads **color + text + shape + position** (colorblind-safe), and a **posture board** showing DRY-RUN / REAL / **`unknown`** (v1).
- **Freshness / as-of board** — the per-source chip in `fresh` / `stale` (1 missed poll) / `critical` (2) / `unknown`.

***Required — addons & the CI gate (machine-checked):***
- **Mandatory addons:** **a11y (axe)**, **viewport**, **interactions** (play functions for the modal / segmented / toggle behaviors), **controls**, **autodocs**.
- **CI gate (blocks merge):** `build-storybook` succeeds **and** the **Storybook test-runner (a11y + interaction)** passes — **axe clean on every story**, every play function green. A component missing its required stories (data-state matrix + a11y-clean) is **not mergeable** (this is the §9 testing gate + the §6 anti-slop lint, enforced in `frontend-ci.yml`).

***Should:*** keep stories small and composable; document props via `controls`; mirror the screen's real layout context (not the bare element) where it aids review.

***Conventions:*** stories are **co-located** with the component; data comes from **fixtures/mocks only** — **never** a live backend or the NyxID proxy; **autodocs** on; **one story = one named state**. Storybook is **publishable as a static design-system site** and **replaces any ad-hoc in-app component gallery route.**

***Not required (out of v1 scope):*** full **screen/page** composites — those live in the app and are covered by RTL + Playwright (§9); Storybook is the **design-system + primitives** surface, not a second copy of the app. **Visual-regression / Chromatic** snapshotting is **optional**, not a v1 merge gate.

**Responsive & mobile (first-class — required, not desktop-only).** Mobile is a supported target. **In-range layout (≥480) ships exactly as the locked DESIGN.md breakpoints specify** (`1080 · 980 · 780 · 600 · 480`); anything **below 480 is an FE extension routed through §11 decision 6**, never adopted as if locked:
- **Touch-first ergonomics.** Tap targets **≥44px** at ≤480; every **hover** affordance has a tap/focus equivalent (nothing hover-only); condense-on-scroll and the freshness chips behave under `pointer: coarse`.
- **The locked behaviors, verbatim from DESIGN.md §5:** the **pipeline never wraps to a 2-row grid** — it **horizontal-scrolls, then stacks to a vertical column**; the **Board** may horizontal-scroll; at **780** the header wraps, **breadcrumb + freshness hide**, and **nav horizontal-scrolls and two-col rows stack**; at **600** bands → 1-col and dense rows inline-wrap; at **480** padding is **16px** and tap targets ≥44px.
- **Sub-480 patterns are NOT adopted by default** — an off-canvas nav drawer, bottom-sheet / full-height modals, dynamic-viewport (`100dvh`) + `env(safe-area-inset-*)` handling, and a compact mobile pipeline are **proposed amendments to the locked design system (§11 decision 6)**, not locked behavior. Until signed off, modals keep their locked semantics (dim backdrop · `--raise` sheet · `--line-2` border · soft seat-shadow · Esc/backdrop/×, per DESIGN.md §7).
- **Verification (§9):** `scrollWidth ≤ clientWidth` is asserted at every locked breakpoint up to the 1440 cap; **360px is an FE-added phone *test* floor** below DESIGN.md's locked 480 (coverage only, not a new visual pattern), and Storybook's viewport addon renders components at phone widths.

**Accessibility (WCAG AA on the dark canvas, colorblind-safe).**
- **Status is never hue-alone** — always color **+ text + shape + position** (state dot + word; CI `✓ / — / ✗`; gated badge = dashed neutral shape; Review = gold rule + "bottleneck" + dot; Ship = red rule + "REAL" + dot).
- **Amber is brand-only** (active window/view segment, `+New goal`, active-filter treatment, **focus-visible ring**) — **never a status hue**.
- **REAL write posture pairs color + text + icon + border + a fixed location** — per DESIGN.md §7's posture chip (REAL = `--red` text + a `color-mix` red border + a red dot), so it **never relies on hue or a dot alone**. In v1 posture is **`unknown`**, so this alarm is simply not asserted (the DRY-RUN/REAL chip reads `posture unknown (deploy-time)`).
- `:focus-visible` amber ring everywhere; honor `prefers-reduced-motion` (no marching dashes, no fake-live animation).
- **Touch a11y:** tap targets **≥44px**, no hover-only affordances, and visible focus under keyboard *and* touch (see **Responsive & mobile**).

**Anti-slop guardrails as lint-able rules** (repo-local ESLint/stylelint + a `design-guard` check):
| Rule | Enforcement |
|---|---|
| No amber as a status | flag amber token in any status/semaphore context |
| No `0` for unknown sources | flag literal `0` in count cells without an "is-reachable" guard |
| Status never hue-alone | require a text/shape sibling next to any status dot/badge |
| No `system-ui` | ban `system-ui` font-family |
| No glows / orbiting logo / marching dashes / decorative gradients/blobs / in-app grid texture / emoji icons / rainbow status / card-mosaic | ban the corresponding classes/keyframes/box-shadow patterns |
| No fake-live | ban infinite "pulse-as-live" animations on data surfaces |
| No per-goal engine control | code-review checklist + a forbidden-action lint on action handlers (see §10) |

---

## 7. Monorepo & build integration

**`frontend/` layout.** The locked design system and this architecture brief now live at **repo-root `docs/`** (`docs/design.md`, `docs/ARCHITECTURE.md`) alongside the API-contract docs; the locked HTML `frontend/mockups/` and the FE **working** docs (`frontend/docs/`: IMPLEMENTATION-PLAN, PENDING, QA-TESTPLAN, VERIFY-REPORT) stay under `frontend/`. The app layout:
```
frontend/
├─ package.json          # CREATE in bootstrap; mirrors root version (below). Its existence ARMS frontend-ci.yml
├─ vite.config.ts
├─ tailwind.config.ts
├─ tsconfig.json
├─ index.html
├─ .storybook/           # Storybook config + addons (a11y/axe, viewport, interactions, controls)
├─ docs/                 # FE WORKING docs: IMPLEMENTATION-PLAN, PENDING, QA-TESTPLAN, VERIFY-REPORT
│                        #   (design.md + ARCHITECTURE.md now live at repo-root docs/)
├─ mockups/              # COPIED IN from .gstack/projects/FKST/designs/goal-board-20260611/ (locked HTML mockups)
├─ src/
│  ├─ app/               # router, providers (QueryClient + persistQueryClient hydrate, theme)
│  ├─ planes/            # github (NyxID proxy client), backend (v1 api), host-agent (deferred stubs)
│  ├─ truth/             # marker-trust, version-ordering, stage bucketing, freshness
│  ├─ features/          # overview, goals, packages, goal-detail, settings (inbox deferred — no route)
│  ├─ components/        # shadcn-skinned primitives + design-system components (each with a .stories.tsx)
│  ├─ styles/            # tokens.css (oklch custom properties)
│  └─ lib/               # query hooks, types; persist/ → IndexedDB persister, dehydrate allow-list, identity-scoped store key + sign-out wipe
└─ tests/                # vitest + playwright
```

**Version mirroring.** Root `package.json` (`name: fkst-hosted`, currently `version: 0.0.0`) is the **single source of truth**, managed by Changesets and described as mirrored into `Cargo.toml` / the frontend `package.json` "once those exist." `frontend/package.json` **mirrors** the root version (a small `scripts/sync-version` step run on release/`sync-release-pr`), and `health.version` (backend crate build) is displayed **only** as the service build version, never as the product version.

**New frontend CI workflow** (`.github/workflows/frontend-ci.yml`), mirroring the proven gate idiom from `rust-ci.yml` / `docker-build.yml`:
- **Trigger:** `pull_request` into `develop` / `develop-auto`.
- **Gate by file existence + automation skip:**
  ```
  if release-automation label   → skip=true (no-op)
  elif [ ! -f frontend/package.json ] → skip=true (no-op, matches the rust/docker pattern)
  else → run: install → lint → typecheck → build → test
  ```
- Steps run when not skipped: `npm ci` → `eslint` (incl. a11y + anti-slop rules) → `tsc --noEmit` → `vite build` → **`build-storybook`** → `vitest run` → **Storybook test-runner (a11y + interaction)** (+ Playwright, incl. **mobile device projects**, in a dedicated job). This keeps green-CI **auto-merge** working (`gh pr merge --auto --merge`) per `CLAUDE.md`.

**Contribution & git discipline (CLAUDE.md — authoritative, applies to every FE change).** Open a **GitHub issue first** → cut a **feature/bugfix branch** (never commit directly to `main`/`develop`/`develop-auto`) → open a **PR linking `Closes #N`** with the repo's issue/PR templates → add a **changeset** (`npx changeset`, pick patch/minor/major) → **auto-merge on green CI** (`gh pr merge --auto --merge`); if CI is red, **fix it then auto-merge**, never hand back a red PR. Keep commits **small and self-contained**. **Never add `Co-Authored-By` or any AI/bot trailer**; always commit and act under the **user's own GitHub identity**. PRs into `main` are review-gated **releases**, not auto-merged.

**Docker / serving story.** Vite emits static assets; the hosted serve is a static origin (CDN or a thin static server / nginx) co-located with the deployment. The backend `Dockerfile` stays backend-only. The FE image (if added) is its own gate-able artifact and must not change backend image behavior. **Storybook** builds to its **own** static bundle (`build-storybook`) — publishable as a design-system site, separate from the app artifact.

**CORS dependency.** The backend currently sets `allow_origin(Any)` (a documented dev-only TODO in `router.rs`). **Tightening CORS to the real FE origin is a backend dependency** the FE must file/track as a backend issue before production; the FE does not work around it with a private side channel.

**Env/config.** Vite `import.meta.env` for `NYXID_BASE`, the hosted backend base URL, and the api-github proxy slug (`api-github`). **No secrets in the FE bundle** — the FE holds only a short-lived NyxID bearer at runtime (§8). The cross-session persistence store (IndexedDB) holds **only non-secret, re-derivable GET reads**, is identity-scoped, and is `buster`-invalidated on an app/schema version bump (§4/§8).

---

## 8. Security & auth posture

- **NyxID PKCE.** Sign-in is **OAuth 2.0 Authorization Code + PKCE** (`@nyxids/oauth-react`, pinned). The SPA is a **public client**; it holds only a **short-lived (~15-min) NyxID bearer** — **never a raw GitHub token, never stored/logged/transmitted.** *(v1: NyxID not integrated → all auth affordances disabled-with-note.)*
- **No token custody / no forwarded auth.** All GitHub I/O goes through `<NYXID_BASE>/api/v1/proxy/s/api-github/<github-path>` with `Authorization: Bearer <nyxid_token>` and **nothing else** — the client must **not** add a second `Authorization` header or pass any GitHub token in body/query. NyxID injects the GitHub credential server-side.
- **Marker trust + version-ordering still run client-side** on whatever the proxy returns (constraints #4/#5) — proxying does not relax trust.
- **Writes only through legitimate channels.** Every mutation is a GitHub mutation via the NyxID proxy, a backend session/package call, or an out-of-band posture/ops change the engine respects — **never by editing redb or the runtime root, never a private side channel.**
- **CORS.** Backend is `Any` for dev; **must be tightened to the FE origin** before production (backend issue, §7).
- **Secrets handling.** None in the bundle; backend secrets/env are deploy-time and out of FE reach (the env panel reads `unknown`).
- **Persisted cache safety.** Cross-session persistence (§4) uses **IndexedDB**, stores **only non-secret, re-derivable GET reads** (never the NyxID bearer — that stays in the SDK's own short-lived store — and never a GitHub token), is **namespaced by the NyxID subject**, **wiped in full on sign-out / identity change — all planes, not just the GitHub partition** — and **`buster`-invalidated** on app/schema version bumps. So a shared machine never hydrates one user's data (issues, runs, package reads) into another's session.

---

## 9. Cross-cutting quality

**Testing.**
| Layer | Tool | Focus |
|---|---|---|
| Unit | Vitest | The `truth/` core: **marker-trust filter**, **version-ordering tuple + tie-breaks**, stage bucketing (12-state → Design/Build/Review/Ship/Blocked/Merged), freshness/`as-of` math, **`unknown`-not-`0`** helpers. |
| Component | RTL | State taxonomy for every data surface (loading/empty/error/unknown); status-never-hue-alone DOM assertions; disabled-control-+-note for each v1 gap; 409/400 inline mapping in modals. |
| e2e | Playwright | **Reuse the backend e2e happy-path contract** (`backend/tests`, the v1 MVP verification): packages list/create, session create → poll → stop, health 200/503. Drive the real Packages screen + Settings hosted-engine pane + per-package stop flow against a running backend, on **desktop + mobile device projects** (a phone + a tablet). GitHub-plane/host-agent e2e are gated until NyxID/host-agent land (assert the honest disabled/empty states now). |
| Design system | **Storybook test-runner (a11y + interaction)** | One story per component × state (loading/empty/error/unknown + disabled-gap); **a11y (axe)** and **interaction (play)** tests run in CI; **viewport** stories at the DESIGN.md breakpoints + phone widths. The canonical, runnable design-system reference. |
| Persistence | Vitest + RTL | Hydrate-from-persisted paints last-known **behind an `as-of` chip, never green/"live"**; **`maxAge`/`buster` eviction**; **identity-scoping + clear-on-sign-out** (no cross-user hydrate); the **dehydrate allow-list** rejects mutations/errors/`unknown`; a failed revalidation of a too-old entry degrades to `unknown`, not a stale number. |

**a11y verification.** jsx-a11y in lint; axe checks in component tests; manual keyboard/focus pass; colorblind simulation for every semaphore (assert text/shape accompanies hue).

**Responsive / overflow verification.** Automated check that on each route `document.documentElement.scrollWidth ≤ clientWidth` at **every locked breakpoint up to the 1440 cap**, plus an **FE-added 360px phone *test* floor** (below DESIGN.md's locked 480 — coverage only, not a new visual pattern), plus assertions that grids use `minmax(0,1fr)` + `min-width:0` (catch the classic blow-out). **Playwright mobile device projects** (a small phone + a tablet) exercise the real screens and assert **≥44px tap targets** and that no affordance is hover-only.

**Performance.** Per-issue marker reads are an N+1 proxy risk — first-paint from the cheap **label hint** (clearly marked *hint*), then **refine from trusted markers** (mark *fact*); never silently present a hint as fact. Lazy-route the heavier screens; React Query dedup + sane `staleTime` keep poll fan-out bounded.

---

## 10. v1 honest-gap matrix

Every gap is grounded in the real v1 API (router = packages + sessions + health; CORS `Any`) and the access model.

| # | Gap (and why) | Exactly how the FE renders it |
|---|---|---|
| 1 | **NyxID not integrated** (no auth; the entire GitHub plane is behind the proxy that doesn't exist yet). | Avatar = **"Sign in with NyxID" disabled + "NyxID integration pending."** All GitHub-plane views (Overview canvas/vitals, Goals Issues, Goal page/modal, board cards, fire/pause label actions, New-goal **Create issue & enable**, "New issue from this", Close PR, View source) render **disabled / empty-state** — never fabricated issues or fake success. Identity/initials = placeholder. |
| 2 | **No posture endpoint** (`FKST_GITHUB_WRITE` is deploy-time env; no read/write route). | Posture reads **`unknown`** everywhere (Ship "REAL" tag, Merging "WRITE: REAL" row, Goal-page callout/merge-gate, Settings verdict, posture chip). **"Arm REAL" / "Set DRY-RUN" / "Write posture →" are DISABLED + note**: *"global FKST_GITHUB_WRITE is set at deploy time; no API to read/change it in v1 (applied via a session restart)."* Never asserts REAL; **never a per-goal pause.** |
| 3 | **No config-read endpoint** (the 8–10 env knobs are host-side). | Settings deployment-knobs panel: each value **`unknown`** in a **disabled read-only field** + *"host-side config, not exposed by the v1 API."* No editable knobs. The 3 GitHub-derived prerequisites read `unknown` until the proxy is reachable. |
| 4 | **Host-agent data deferred** (redb ledger/DLQ, logs, runtime topology; not reachable via NyxID or the hosted API; engine is hosted not local). | Goals **Activity** view, Goal-page **Deliveries·redb** + **Runs·codex** panels, Activity vitals, **in-DLQ** = **disabled / "unknown" / "host telemetry not connected."** **No requeue / no replay-DLQ / no run mutation** controls ever rendered. in-DLQ is the canonical **`unknown`-not-`0`**. |
| 5 | **No runs-journal deep link** (`SessionView` omits `run_key`; journal coords unresolvable from the API). | "Open run journal on GitHub" **disabled + "run_key not exposed by v1 API."** Don't invent a journal URL. |
| 6 | **Packages support create/update/delete; no per-goal/per-session control beyond create/stop; "Apply changes" is not an endpoint.** | **No edit/delete in UI** (update/delete available via API; UI coming soon). **Per-package enable toggle is target-state intent only** (no enable endpoint / no stored flag) — disabled-or-clearly-intent + note. **"Apply changes / stop & restart" = client-orchestrated** stop→poll-stopped→create, progress shown across three calls. **No reopen/resume/retry** on terminal goals; **no per-goal pause.** |
| 7 | **`/health` `version` = backend crate build**, not product version; **`/health` does not report GitHub freshness.** | Show `version` labeled "backend build." GitHub freshness chip is **FE-tracked from the last proxy fetch**, never from `/health`. |
| 8 | **Conformance is not a standalone endpoint** (only a terminal session `failed` + `error`). | Package conformance renders **from the latest session for that package**: reached `running` ⇒ passed; `failed` with a conformance error ⇒ failing badge + the error text; otherwise **`unknown`**. **Never a standalone green check without session evidence.** |
| 9 | **Scope = "all deployments" / graph selector** (single deployment; no registry endpoint). | Single **inert/disabled** option + note ("one hosted deployment in v1"). |
| 10 | **No deployment-wide suspend** (leases are per package — `LeaseDoc._id == package_name`; `stop` ends one package's session). | A "Suspend deployment" affordance, if shown at all, is **disabled + "v1 sessions are per-package; no deployment-wide suspend endpoint."** The real, available control is the per-package **"Stop session for `<package>`"** (`stop` → poll `stopped`). |
| 11 | **"Intake · raised 2m ago" raiser-tick freshness** (host-agent only). | Show the **static 5-min cadence**; the live "raised Nm ago" reads **derived-or-unknown**, never asserted live. |

**Forbidden actions that must never have a control** (constraint, enforced by §6 lint + review): mutating redb ledger/DLQ ("requeue"); resuming/reopening/retrying a **terminal** goal; a **per-goal pause** (writes are global posture); a **deployment-wide suspend** (no endpoint; only per-package stop exists); "connect a local host agent." Each terminal-goal re-engagement is **only** "New issue from this" (capability #1).

---

## 11. Risks, assumptions & open decisions

**Assumptions.**
- v1 ships as a React SPA in `fkst-hosted` with NyxID-brokered auth and an optional/deferred host agent; the product is **ChronoAI-hosted** (a self-hosted/BYO-engine variant could reintroduce a local agent — out of v1 scope).
- The backend contract is frozen as observed (router = packages + sessions + health; packages support create/update/delete; one-live-session-per-package keyed by package name; CORS `Any`). The FE adapts; it does not pressure the engine to grow APIs.

**Risks.**
- **N+1 proxy fan-out** for per-issue marker history (in/out counts, median time-in-review). Mitigation: cheap label first-paint (marked *hint*) → refine from markers (*fact*); cap concurrency; bounded `staleTime`.
- **CORS tightening is a backend dependency** — production needs the real origin; the FE must not work around a private channel.
- **Honesty regressions** (a stray `0`, an amber status, a fake-live pulse, a phantom requeue button, a "suspend deployment" that only stops one package) are the highest-severity FE bugs here — hence the lint rules + review checklist in §6/§10.
- **Stale persisted data read as live.** Mitigation: every hydrated value paints behind its `as-of` ("last synced … · revalidating"), `maxAge`-evicts, `buster`-invalidates on a version bump, and degrades to `unknown` rather than re-presenting an old number (§4).
- **Shared-machine cache leakage.** Mitigation: the persistence store is identity-scoped and wiped on sign-out; GitHub-plane reads persist only under a NyxID-subject key (§8).
- **redb reader contradiction (host-agent, when it lands):** the engine holds `delivery.redb` under an exclusive lock, so a future reader must **snapshot-copy then open the copy read-only** and verify `meta.schema_version`. (Deferred, but design for it.)

**Genuinely open decisions (from the TRD — resolve before the relevant build).**
1. **Observatory-first vs control-from-day-one.** Recommendation: ship the read-only observatory; add control behind explicit posture confirmation once the read model is trusted.
2. **Host agent at launch (Tier 2) vs GitHub-plane-only start (Tier 1).** In the hosted model these internals move to the hosted backend, but whether v1 serves them is open — the task brief marks the plane **deferred**.
3. **Diagnostic-feed retention/observability** — logs are scratch and may rotate; the FE must degrade gracefully when traces are gone.
4. **NyxID items to confirm before wiring:** (a) the broker-authorization scope that lets a public-client login token proxy the user's `api-github` credential; (b) **CORS on `/api/v1/proxy/...` for the FE origin** (else front it with a thin same-origin pass-through); (c) `@nyxids/oauth-react` availability + a version pin.
5. **Eng Review still required** before implementation (only Design Review has run); resolve the redb concurrent-read/exclusive-lock handling and the NyxID proxy-token/CORS authorization first.
6. **Sub-480 mobile patterns.** DESIGN.md is locked and defines responsive down to 480 (tap ≥44px). Any phone-specific pattern beyond it (off-canvas nav drawer, bottom-sheet modals, a compact mobile pipeline) is an **amendment to the locked design system**, not an ad-hoc FE invention — resolve with design before building those.
7. **Persistence `maxAge`/staleness values (the *invariant* is fixed; the *numbers* are tunable).** The ordering invariant is locked — **`maxAge` ≤ the critical-staleness cutoff**, staleness judged at hydrate time, reachability-gated degrade (§4) — but the exact `maxAge`, the critical cutoff (against the ~5-min cadence), and the per-plane allow-list are pinned during **M2**. IndexedDB via `idb-keyval` is the assumed persister.

---

*This brief is the FE's contract with reality. When a screen wants something the v1 backend doesn't serve, the answer is one of: render it from the GitHub plane (NyxID), render it as an honest disabled gap, file it upstream as a backend issue — **never fake it.***
