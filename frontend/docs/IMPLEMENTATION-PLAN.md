# fkst-hosted Frontend Implementation Plan

> **Status:** PR-by-PR roadmap for `frontend/`. Authoritative once approved alongside
> [`ARCHITECTURE.md`](./ARCHITECTURE.md). It builds the seven locked mockups
> (`frontend/mockups/`) against the **real** hosted backend v1 API (this repo, `develop`),
> the GitHub plane (via NyxID — a v1 gap), and the deferred host-agent plane, under
> [`../../CLAUDE.md`](../../CLAUDE.md)'s git/changeset/auto-merge discipline.
>
> Where this plan and an aspirational doc conflict, the **real v1 API + CLAUDE.md win**.
> No invented endpoints: every increment maps to `packages` / `sessions` / `health`,
> the GitHub plane, or an honest disabled gap.

---

## A. Guiding principles

These bind **every** increment below. They are not aspirational — they are the merge gate.

1. **No app code until this plan + the architecture brief are approved.** Today `frontend/`
   holds only reference artifacts (`docs/design.md`, `docs/ARCHITECTURE.md`, this plan,
   `mockups/`, `README.md`). There is **no `frontend/package.json` and no React app yet**,
   and the frontend CI workflow is intentionally a no-op until that gate file exists.
   Increment **F0a** is the first one allowed to write code, and only after approval.

2. **Issue-first git discipline (CLAUDE.md, authoritative).** Every increment is exactly:
   **open a GitHub issue first** (using `.github/ISSUE_TEMPLATE/` — `feature_request.md`
   for features, `bug_report.md` for defects) → cut a **feature/bugfix branch** (never
   commit directly to `main`/`develop`/`develop-auto`) → open a **PR into `develop`**
   (or `develop-auto`) with the repo PR template, body containing **`Closes #N`** →
   add a **changeset** (`npx changeset`, pick the bump named in the row) →
   **auto-merge on green CI** (`gh pr merge --auto --merge`). If CI is red, **fix it then
   auto-merge** — never hand back a red PR.

3. **Small, self-contained PRs.** Each row in §C is one PR, reviewable in a single sitting.
   If a row grows past that, split it — and update its dependency edges here.

4. **Honesty-first (the highest-severity bug class on this product).** A stray `0` for an
   unreachable source, an amber-as-status, a fake-live pulse, a phantom "requeue" button,
   a "suspend deployment" that only stops one package, or asserting `REAL` posture the FE
   cannot read — each is a **release-blocking** regression. Unreachable → `unknown`,
   never `0`. v1 gaps render as a **disabled control + an honest note**, never fictional
   success. These are enforced by the anti-slop lint rules (increment F2) and the review
   checklist (§E), not just documented.

5. **Scope boundary.** Only user-facing/public interfaces. **Never** the kernel engine;
   `fkst-substrate` and `fkst-packages` are read-only references. If a screen needs a read
   the backend doesn't serve, the answer is: render from the GitHub plane, render an honest
   disabled gap, or **file it upstream as a backend issue** — never fake it, never grow the
   API from here.

6. **User identity only.** Every commit and `gh` action runs under the human maintainer's
   own GitHub identity. **Never add `Co-Authored-By` or any AI/bot trailer.**

7. **PRs into `main` are review-gated releases**, not auto-merged. Only `develop` merges
   into `main`. The increments below all target `develop`/`develop-auto`.

---

## B. Milestones (M0..M6)

A phased path from an empty folder to a v1 console that is truthful on the real backend and
honestly disabled everywhere a plane is not yet wired.

| Milestone | Theme | Exit criterion |
|---|---|---|
| **M0 — Scaffold** | Vite + React 18 + TS (strict) toolchain (F0a) → `frontend-ci.yml` gate armed (F1) → app shell with topbar/nav/condense-on-scroll behind the green gate (F0b), Tailwind themed from the locked oklch tokens + anti-slop/a11y lint (F2), router, `QueryClient` provider, version mirror, frontend README updates. | `frontend/package.json` exists from **F0a** and **arms** `frontend-ci.yml` (which then **runs, not no-ops**, on **F1's own PR**); CI runs `install → lint → typecheck → build → test` green; the app shell (F0b) renders the topbar + empty routes for Overview/Goals/Packages/Settings; **no rust/docker CI behavior changed**. |
| **M1 — Design-system primitives** | Token CSS, Tailwind theme map, the three fonts, shadcn/Radix primitives skinned to tokens, the locked component vocabulary (freshness chip, state dot, badges, list/row `.plist`/`.levels`/`.rw`, segmented controls, posture chip, window control, vitals cell, modal shell), anti-slop + a11y lint rules. | A Storybook-or-route component gallery renders each primitive in loading/empty/error/unknown; `eslint` (jsx-a11y + repo anti-slop rules) passes; status-never-hue-alone and `unknown`-not-`0` are lint-enforced. |
| **M2 — Data layer + backend v1 client** | Typed clients + TanStack Query hooks for the **only** directly-called backend: `useHealth`, `usePackagesList`, `usePackage`, `useCreatePackage`, `useCreateSession`, `useSession` (terminal-aware polling), `useStopSession`; the `truth/` core (marker-trust, version-ordering tuple, stage bucketing, freshness/as-of math, `unknown`-not-`0` helpers) with Vitest coverage. Env wiring (`NYXID_BASE`, backend base URL, `api-github` slug). | All hooks typed against `SessionView` / `PackageResponse`; `truth/` unit tests green (trust filter, version tuple + tie-breaks, 12-state → stage bucket, `unknown`-not-`0`); session polling stops on terminal; nothing animates as live. |
| **M3 — Read-only screens on the REAL backend** | The surfaces the v1 backend genuinely serves: **Packages** (list/detail/create + topology-from-`files[]` + read/write tri-panel), the **Settings hosted-engine pane + per-package Stop**, the **New-goal modal's read-only package graph**, and the chrome/shell of every GitHub-plane screen rendering honest skeleton/empty/disabled. The **Apply-changes** client-orchestrated stop→poll→create flow. | Packages list/detail/create work against a running backend (incl. 409/400 inline); session create→poll→stop works; conformance is session-derived (never a standalone green check); every GitHub-plane screen renders a truthful disabled/empty state; the backend **e2e happy path stays green**. |
| **M4 — GitHub plane via NyxID** | NyxID PKCE login (`@nyxids/oauth-react`), the `api-github` proxy client, marker trust + version-ordering applied to live data, then the GitHub-derived screens lit up: Overview Pipeline, Overview Board, Goals Issues, Goal page/modal lifecycle, vitals, Needs-you. | Sign-in works; GitHub-plane screens render real poll-derived data with per-source freshness chips; the FE never holds a raw GitHub token; the GitHub freshness chip is FE-tracked from the last proxy fetch (never from `/health`). **Gated on the §F NyxID confirmations.** |
| **M5 — Actions, posture & honest gaps** | The four legitimate GitHub mutations (create-issue/enable, label, comment, close PR) via the proxy; the New-goal create wired; posture rendered honestly (`unknown` everywhere, elevation flow disabled-with-note); the forbidden-action lint passing. | Every action maps to exactly one real capability; no forbidden control exists anywhere (requeue / resume-terminal / per-goal pause / deployment-wide suspend / connect-local-agent); posture never asserts `REAL`. |
| **M6 — Host-agent / advanced (DEFERRED)** | Activity (folded Runs) view, redb/codex diagnostic panels — built only if/when a host-agent or backend journal/runs read endpoint lands. Until then they ship as the honest "host telemetry not connected" / "unknown" disabled sections specified in M3. | Out of v1 scope; tracked as upstream backend issues. No fabricated rows, no requeue/replay controls ever. |

---

## C. PR-sized increment backlog

The heart of the plan. **One row = one PR.** Ordered so dependencies precede dependents.
`Dep` is by increment ID. `Bump` is the changeset SemVer bump. `Plan?` = parallelizable
with its siblings once deps are met. Every row opens its own GitHub issue (one-line,
template-faithful) and links it `Closes #N`.

### Milestone M0 — Scaffold

| ID | Title | Issue (template-faithful) | Scope / deliverable | Dep | Bump | Plan? | Backend/plane touched | v1 honesty handling | Acceptance |
|---|---|---|---|---|---|---|---|---|---|
| **F0a** | Scaffold the toolchain + gate file + empty router/provider | *Feature:* "Scaffold the `frontend/` Vite + React 18 + TS toolchain (gate file, empty router, QueryClient, Vitest) so the FE CI gate can arm." | Create `frontend/package.json` (the **gate file**, mirrors root version), `vite.config.ts`, `tsconfig.json` (strict), a **minimal** `index.html`, `src/main.tsx` + `src/app/` with an **empty React Router data router** (routes `/`→`/overview`, `/overview`, `/goals`, `/packages`, `/settings` mapped to bare placeholder elements; **no `/inbox`, no `/runs`** yet) and the `QueryClient` provider. Wire **Vitest** with `passWithNoTests` (or one trivial passing smoke test) so `vitest run` is green from the first commit. **No topbar/nav shell, no condense-on-scroll, no redirects yet** — those land in F0b behind the armed gate. | — | minor | No | none (toolchain only) | No fake data anywhere; placeholder route elements are bare ("screen pending"), not fabricated content. | App boots to `/overview` with a bare placeholder; `npm ci → eslint → tsc --noEmit → vite build → vitest run` all pass locally; no console errors. **F0a must be green on lint/typecheck/build/test so F1 can merge green.** |
| **F1** | Add the `frontend-ci.yml` gate workflow | *Feature:* "Add a file-existence-gated frontend CI workflow (install/lint/typecheck/build/test) mirroring `rust-ci.yml`." | `.github/workflows/frontend-ci.yml`: `pull_request` into `develop`/`develop-auto`; skip on `release-automation` label **or** when `frontend/package.json` is absent (the proven gate idiom from `rust-ci.yml`/`docker-build.yml`); else `npm ci → eslint → tsc --noEmit → vite build → vitest run` (Playwright in a dedicated job, M3+). | F0a | patch | No | none (CI) | The **else-branch is the live path** from this PR onward (F0a already added the gate file); the no-op idiom only protects pre-F0a/`release-automation` PRs. Does not alter rust/docker workflows. | Because F0a landed first, `frontend/package.json` is present, so on **F1's own PR** the workflow **runs (not no-ops)** the full `install/lint/typecheck/build/test` against the F0a app and is **green** (relies on F0a's passing test / `passWithNoTests`); on a `release-automation` PR it no-ops; rust/docker CI unchanged. |
| **F0b** | App shell: topbar/nav + condense-on-scroll + placeholder bodies + redirects | *Feature:* "Add the topbar/nav app shell (condense-on-scroll hysteresis 140/40), honest placeholder route bodies, and the `/runs`→`/goals?view=activity` redirect." | The **topbar/nav shell** with condense-on-scroll (hysteresis 140/40 — a tested behavior per DESIGN.md §61) wrapping the F0a router outlet; honest "screen pending" placeholder bodies for `/overview` `/goals` `/packages` `/settings`; the `/runs`→`/goals?view=activity` redirect (`/inbox` still not registered). Lands **after** the gate so its CI exercises install/lint/typecheck/build/test on this component. | F0a, F1 | patch | No | none (shell only) | Routes for unbuilt screens render an honest "screen pending" placeholder, not fake data. `/runs`→`/goals?view=activity` redirect; `/inbox` not registered. | App boots; `/overview` renders the topbar + nav; condense-on-scroll toggles at the 140/40 thresholds (unit-tested with the hysteresis assertion); `scrollWidth ≤ clientWidth` at the 1440 cap; no console errors; **frontend-ci runs green on this PR**. |
| **F2** | Token CSS + Tailwind theme + fonts + anti-slop/a11y lint | *Feature:* "Wire the locked oklch design tokens into Tailwind, load the three fonts, and add anti-slop + a11y lint rules." | `src/styles/tokens.css` (oklch custom properties from `docs/design.md`), `tailwind.config.ts` mapping theme→vars, Space Grotesk / IBM Plex Sans / IBM Plex Mono (Plex Sans for UI, **never `system-ui`**), ESLint (typescript-eslint, jsx-a11y, react-hooks) + Prettier + repo-local rules (no amber-as-status, no `0`-for-unknown, status-never-hue-alone, ban `system-ui`/glows/marching-dashes/fake-live, forbidden-action lint stub). | F0b | patch | No | none | The lint rules are the machine enforcement of §A.4 honesty. | `eslint` flags a deliberate amber-status fixture and a bare-`0`-count fixture in tests; build resolves all utilities to tokens. |

### Milestone M1 — Design-system primitives

| ID | Title | Issue | Scope / deliverable | Dep | Bump | Plan? | Plane | Honesty | Acceptance |
|---|---|---|---|---|---|---|---|---|---|
| **F3** | shadcn/Radix primitives skinned to tokens | *Feature:* "Add token-skinned Radix primitives (Dialog, Dropdown/Select, Switch, Tabs/segmented, Tooltip, Toast)." | shadcn/ui (Radix) Dialog, Dropdown/Select, Switch, Tabs/segmented, Tooltip, Toast — restyled to tokens; **no library default visual identity ships**. `:focus-visible` amber ring; `prefers-reduced-motion` honored. | F2 | patch | Yes | none | Switch supports a **disabled** posture/enable state for v1 gaps. | Each primitive renders in the gallery, keyboard-navigable, focus ring amber; axe passes. |
| **F4** | Status & freshness component vocabulary | *Feature:* "Add the locked status/freshness components: state dot, 12-state badge, CI glyph, freshness chip, posture chip, vitals cell." | State dot (color+text+shape+position), 12-state badge (`.gated` dashed neutral), CI glyph (`✓`/`—`/`✗`), **GitHub freshness chip** (FE-tracked as-of), **posture chip** (REAL = `--red` text + red border + red dot; in v1 reads `posture unknown (deploy-time)`), vitals cell. | F3 | patch | Yes | none | Posture chip **cannot assert REAL** in v1; default state is `unknown`. Status never hue-alone (lint-asserted). | Each renders loading/empty/error/`unknown`; an `unknown` count shows the literal token, never `0`; colorblind-sim test asserts text/shape accompanies hue. |
| **F5** | List/row + layout primitives (`.plist` / `.levels` / `.rw`, window/view segmented) | *Feature:* "Add the hairline list/row pattern, the Company·Dept·Person levels grid, the read/write tri-panel, and the window/view segmented controls." | `.plist`/`.levels`/`.rw` hairline grid (not bordered cards), package row, window segmented control (Live/1h/24h/7d/30d), view-switch segmented control, issue-modal shell. **All grids use `minmax(0,1fr)` + `min-width:0`** (no plain `1fr`). | F3 | patch | Yes | none | Amber on active window/view segment is **brand selection**, never status. | Overflow test: `scrollWidth ≤ clientWidth` at breakpoints up to 1440; modal traps focus and closes on Esc/backdrop. |

### Milestone M2 — Data layer + backend v1 client

| ID | Title | Issue | Scope / deliverable | Dep | Bump | Plan? | Backend endpoints | Honesty | Acceptance |
|---|---|---|---|---|---|---|---|---|---|
| **F6** | Backend v1 typed client + env wiring | *Feature:* "Add the typed hosted-backend v1 client (`SessionView`, `PackageResponse`) and `import.meta.env` config." | `src/planes/backend/` client + types; `import.meta.env` for `NYXID_BASE`, backend base URL, `api-github` slug; **no secrets in the bundle**. Error mapping scaffold (409/400/404/503). | F0a | patch | No | all `/api/v1/*` (typed surface) | `health.version` typed/labeled as **backend build**, never product version. | Types compile against the real wire shapes; a unit test asserts 503 maps to `degraded`/`unknown`, never `0`. |
| **F7** | `useHealth` + degraded banner | *Feature:* "Add `useHealth()` (short staleTime) and a discreet degraded banner keyed off 200 vs 503." | `useHealth()`; degraded note from `status==='degraded' || HTTP!==200` (key off **status code**, not just body). Drives the hosted-engine health surface — **not** the GitHub freshness chip. | F6, F4 | patch | Yes | `GET /api/v1/health` | Mongo-down → discreet note; never blanks the screen; never feeds the GitHub chip. | Banner shows on a 503; chip stays neutral; `version` rendered as "backend build". |
| **F8** | `truth/` core — marker trust + version ordering | *Feature:* "Implement the client-side marker-trust filter and the version-ordering tuple with tie-breaks." | `src/truth/`: trust filter (honor only `FKST_GITHUB_BOT_LOGIN`-authored markers; neutralize untrusted `<!-- fkst:`); version-ordering by `(updated_at, loop_n, fix_n, review_meta_action_n, review_loop_n, stage_rank)`, tie-break `blocked > ready`; **pure functions, no I/O**. | F6 | patch | Yes | none (operates on proxy output later) | "Most recent comment" is **not** current state; bot-login unreadable in v1 → generic "trusted-bot" chip. | Vitest: trust filter rejects non-bot markers; version tuple + tie-breaks match fixtures; current-state = max marker. |
| **F9** | `truth/` core — stage bucketing + freshness/`unknown` helpers | *Feature:* "Implement 12-state→stage bucketing, freshness/as-of math, and the `unknown`-not-`0` helpers." | 12-state → Design/Build/Review/Ship/Blocked/Merged buckets; terminality flags (`impl-failed`/`blocked`/`merged` terminal — no out-edges); per-source `as-of` + stale thresholds (warn after 1 missed poll, critical after 2); `unknown`-not-`0` count helper. | F8 | patch | Yes | none | Helper forces `unknown` for any unreachable deciding source; terminal goals expose **no console out-edges**. | Vitest: each state buckets correctly; terminal flags correct; `count(unreachable)` returns `unknown`, `count(reachableEmpty)` returns `0`. |
| **F10** | Package + session query hooks (terminal-aware polling) | *Feature:* "Add `usePackagesList`, `usePackage`, `useCreatePackage`, `useCreateSession`, `useSession`, `useStopSession` with terminal-aware polling." | TanStack Query hooks; `useSession` `refetchInterval` while non-terminal, **stops** on `stopped`/`failed`; `useCreatePackage`/`useCreateSession` mutations with 409/400 mapping; 201 invalidates the list. | F6 | patch | No | `GET/POST /api/v1/packages`, `GET /api/v1/packages/:name`, `POST/GET /api/v1/sessions`, `POST /api/v1/sessions/:id/stop` | Session polling never animates as live; stop's `202` is an ack only — truth is the subsequent GET. | Component tests: polling stops on terminal; 409 surfaces inline; stop keeps polling until `stopped`. |

### Milestone M3 — Read-only screens on the REAL backend

| ID | Title | Issue | Scope / deliverable | Dep | Bump | Plan? | Backend/plane | Honesty | Acceptance |
|---|---|---|---|---|---|---|---|---|---|
| **F11** | Packages screen — list + detail + flat/composed/deps | *Feature:* "Build the Packages loaded-list (flat/composed badges, deps chips, conformance, intro + Company·Dept·Person)." | `/packages` list from `usePackagesList`, lazy `usePackage` per row; flat/composed badge = `composed_deps.length>0`; deps chips; intro lede + levels tri-cell; section counts. | F10, F4, F5 | minor | No | `GET /api/v1/packages`, `GET /api/v1/packages/:name` | `[]`→real empty state; `updated_at`==`created_at` → never "edited"; conformance/role/namespace/dept-counts render `unknown` (not parsed yet). **No edit/delete affordance.** | List/detail render against a live backend; empty store shows the honest empty row; no edit/delete buttons exist. |
| **F12** | Packages screen — topology from `files[]` + read/write tri-panel | *Feature:* "Render the composed topology (sources→departments→queues) FE-derived from `files[]`, and the read/write boundary tri-panel." | Parse `files[].path` for `departments/*/main.lua` + `raisers/*.lua` to derive the SOURCES band, department rows, queue chips, codex tags; the `.rw` tri-panel (source read-only · FE manages set/topology/posture via restart · writes→GitHub under REAL). | F11 | patch | Yes | `GET /api/v1/packages/:name` (`files[]`) | Topology is **FE-derived** and labeled poll-derived/scanned-at-startup; where parse is unavailable → `unknown`, never `0`. "scan-once · no hot-reload · source read-only at runtime" copy is load-bearing. | Topology renders from a real package's files; the tri-panel copy is present; freshness chip says "scanned at startup". |
| **F13** | Packages — +Add package modal (create-only) | *Feature:* "Add the +Add package modal: POST create-only with structure-only validation, 409/400 inline." | `useCreatePackage` in the modal; Zod mirrors structure-only rules (name `^[A-Za-z0-9_-]+$`, ≥1 file, ≤256 files, ≤1MiB/file, ≤12MiB total, must contain an engine entry); 201→invalidate; **409 inline** "name already exists (a revision is a new name)"; 400→field-level. | F11 | patch | Yes | `POST /api/v1/packages` | Create alone does **not** load the package (composes on next session start); server is final authority. | Submitting a dup name shows the 409 inline message; a structural error maps to the field; 201 refreshes the list. |
| **F14** | Per-package enable toggle (target-state intent) + Apply-changes flow | *Feature:* "Render the per-package enable toggle as target-state intent and the client-orchestrated Apply-changes (stop→poll→start)." | Enable toggle = **disabled-or-clearly-intent + note** (no enable endpoint / no stored flag); "Apply changes · stop & restart session" = `POST stop → poll GET until stopped → POST /sessions {package_name}`, progress shown across the three calls; **per-package** "Stop session for `<package>`". | F13, F10 | patch | Yes | `POST /api/v1/sessions/:id/stop`, `GET /api/v1/sessions/:id`, `POST /api/v1/sessions` | Toggle never implies success; the lease is **per package** (`LeaseDoc._id == package_name`) — no deployment-wide suspend; 409 → "already has a live session — stop it first". | Apply-changes runs the three calls and shows progress; toggle is non-persisting with a note; stop acts on one package only. |
| **F15** | Settings — hosted-engine pane + per-package Stop (the truthful v1 part) | *Feature:* "Build the Settings hosted-engine connection pane (health + session status) and the per-package Suspend (Stop) control." | `/settings` hosted-engine pane from `useHealth` + `useSession`; replica/lease-fenced topology as deploy-fact copy; **"Stop session for `<package>`"** confirm→`POST stop`→poll. The one genuinely mutating control on Settings. | F10, F7 | minor | No | `GET /api/v1/health`, `GET /api/v1/sessions/:id`, `POST /api/v1/sessions/:id/stop` | Unreachable engine → `unknown`, never "live"/`0`. **"Suspend deployment"** affordance, if shown, is **disabled + "v1 sessions are per-package; no deployment-wide suspend endpoint."** | Engine pane reflects real health/session; Stop works; no deployment-wide suspend is functional. |
| **F16** | Settings — posture/knobs/identity honest gaps | *Feature:* "Render Settings write-posture, deployment knobs, and identity/connections as disabled-with-note v1 gaps." | Posture verdict reads `unknown` (no posture endpoint); "Arm REAL"/type-to-confirm/"Enable REAL writes" **disabled + grounding note**; 10 deployment-knob rows read `unknown` in disabled read-only fields ("host-side config, not exposed by the v1 API"); identity/repos/Sign out/Disconnect/Reconnect/Connect-repo/Delete-account **disabled + "NyxID integration pending"**; the 3 GitHub-derived prerequisites read `unknown`. | F15, F4 | patch | Yes | none (gaps) + `GET /api/v1/health` | Never asserts `0`/DRY-RUN-by-default; the confirm box disowns a fictional per-goal pause/cancel-mid-merge; poll-cadence is a **link to Packages**, not a setter. | Every gap control is disabled with the exact grounding note; posture shows `unknown`; knobs show `unknown`. |
| **F17** | New-goal modal — read-only package graph (backend-served) + disabled submit | *Feature:* "Build the +New goal modal: read-only deployment package graph from the backend, with submit disabled pending NyxID." | The modal shell + the **read-only package graph** (`GET /api/v1/packages` + `/packages/:name`→`composed_deps`); repo select seeded/static; "Create issue & enable" **disabled + 'requires NyxID sign-in'**. Reusable from Overview, Goals, and the Goal page. | F11, F5 | patch | Yes | `GET /api/v1/packages`, `GET /api/v1/packages/:name` | The only backend-served part of the modal; the create is a GitHub write (capability #1) — disabled until M4. | Modal opens from all three entry points; graph renders from the backend; submit is disabled with the note. |
| **F18** | GitHub-plane screen shells (honest skeleton/empty/disabled) | *Feature:* "Render the Overview / Goals / Goal-page shells as honest GitHub-plane gaps (skeleton/empty + 'Sign in with NyxID' disabled)." | Build the **chrome + layout** of Overview (Pipeline + Board toggle, vitals cells, Needs-you band), Goals (Issues table, Issues/Activity toggle), and the Goal page (decision header, timeline rail, diagnostics rail) — all rendering the **loading→empty/disabled** states (no fabricated issues). Avatar = "Sign in with NyxID" disabled. `/goals?view=activity` Activity = "host telemetry not connected". | F5, F4 | minor | No | none (GitHub plane + host-agent, both gaps) | First poll is *loading*, never animated live; counts read `—`/`unknown`, never `0`; freshness chip "syncing…"/neutral. | Each screen renders its full chrome with truthful disabled/empty bodies; no fake data; lint passes (no fake-live). |

### Milestone M4 — GitHub plane via NyxID

> **Gated on the §F NyxID confirmations** (broker scope, proxy CORS for the FE origin,
> `@nyxids/oauth-react` availability + version pin). Do **not** start F19 until resolved.

| ID | Title | Issue | Scope / deliverable | Dep | Bump | Plan? | Plane | Honesty | Acceptance |
|---|---|---|---|---|---|---|---|---|---|
| **F19** | NyxID PKCE login + identity | *Feature:* "Integrate NyxID OAuth2 Authorization-Code + PKCE; SPA holds only a short-lived bearer." | `@nyxids/oauth-react` (pinned); Sign-in/Sign-out; avatar identity from `getUserInfo()`; **no raw GitHub token, never stored/logged**. | F18 | minor | No | NyxID | The SPA never holds a raw GitHub token (footer invariant). | Login round-trips; avatar shows real initials; no GitHub token anywhere in storage/logs. |
| **F20** | `api-github` proxy client | *Feature:* "Add the NyxID `api-github` proxy client (single Bearer, no second Authorization header)." | `src/planes/github/` client → `<NYXID_BASE>/api/v1/proxy/s/api-github/<github-path>` with **only** `Authorization: Bearer <nyxid_token>`; **GitHub freshness chip FE-tracked from the last successful proxy fetch**; bounded concurrency for per-issue marker reads. | F19, F8, F9 | patch | No | GitHub plane | Marker trust + version-ordering run client-side on proxy output; chip is **never** fed by `/health`. | Proxy calls succeed with one Bearer; freshness chip tracks the last fetch; trust/ordering applied. |
| **F21** | Goals — Issues view (live) | *Feature:* "Light up Goals Issues: poll issues + per-issue markers, derive state/stage, render the hero table." | Issues list via the proxy; per-issue markers refine the dot/stage/badge; filters/window/search client-side; **cheap label first-paint (hint) → refine from markers (fact)** to bound N+1. | F20, F18 | minor | Yes | GitHub plane | Labels are **hints**, markers are **fact** (provenance line); CI `unknown`→`—`, never a pass; "Live" is a poll cadence, not a stream. | Real issues render with marker-derived state; hint→fact refine visible; per-source as-of present. |
| **F22** | Overview — Pipeline view (live) | *Feature:* "Light up the Overview Pipeline rail (Intake end-cap · Design/Build/Review/Ship · Merged end-cap) with vitals." | Stage columns + in/out + 2-line goal rows; vitals (in-flight, merged 24h, dead-ended, throughput `~/h`, median time-in-review); Review bottleneck = gold rule+text+dot. | F21 | minor | Yes | GitHub plane | Ship "REAL" tag → `posture unknown (deploy-time)`; Intake "raised Nm ago" → derived-or-`unknown`, 5m cadence static; gold/red rules pair color+text+dot. | Pipeline renders from live data; vitals are poll-derived with `~`/`/h` qualifiers; posture never asserted. |
| **F23** | Overview — Board view + Needs-you | *Feature:* "Add the Overview Board (kanban by stage) and the Needs-you band (terminal + REAL-write items)." | Board columns of goal cards; Needs-you = terminal blocked/impl-failed + in-flight merging rows, each with **exactly one** grounded action; "New issue from this" on terminal rows. | F22 | minor | Yes | GitHub plane | Terminal re-engagement = **new GitHub fact** only; Merging "WRITE: REAL" → `unknown`; no per-goal pause; empty → "Nothing needs you". | Board + Needs-you render from live data; terminal rows offer only "New issue from this"; no reopen/resume/retry. |
| **F24** | Goal page + Issue modal — lifecycle (live) | *Feature:* "Light up the Goal page / Issue modal: trust-annotated lifecycle timeline + diagnostics from GitHub." | Decision header (the ONE decision), version-ordered timeline with trust + as-of + raw markers, merge-gate + PR-diff review panels from GitHub; the host-agent Deliveries·redb / Runs·codex panels stay **disabled/`unknown`** (M6). | F20, F18 | minor | Yes | GitHub plane (host-agent panels deferred) | `● NOW · REAL` pairs red+text+shape+position; posture `unknown`; "stale replay skipped" surfaced (CAS skip-stale); Runs is "diagnostic", not a status source. | Timeline renders version-ordered with trust chips; diagnostics from GitHub; redb/codex panels show the honest deferred state. |

### Milestone M5 — Actions, posture & honest gaps

| ID | Title | Issue | Scope / deliverable | Dep | Bump | Plan? | Plane | Honesty | Acceptance |
|---|---|---|---|---|---|---|---|---|---|
| **F25** | New-goal create + label actions (the legitimate GitHub mutations) | *Feature:* "Wire the four legitimate GitHub mutations via the proxy: create-issue/enable, label, comment, close PR." | Enable the New-goal "Create issue & enable" (`POST issues` + `POST labels {fkst-dev:enabled}`); fire/pause via label add/remove; "New issue from this"; "Close PR" (`PATCH pulls/{n} state=closed`). Post-action copy = "Requested via NyxID at HH:MM; engine effect expected next ~5-min tick" — never instant-success. | F19, F20, F23, F24 | minor | No | GitHub plane | Each action = exactly one capability; **forbidden controls absent** (requeue / resume-terminal / per-goal pause / deployment-wide suspend / connect-local-agent) — enforced by the forbidden-action lint. | Create/label/comment/close-PR work via the proxy under the user's account; no forbidden control renders; post-action copy is honest. |
| **F26** | Posture rendering & elevation flow (disabled-with-note) | *Feature:* "Render write posture honestly everywhere (`unknown`) and the REAL elevation flow as a disabled gap." | Posture chip/verdict reads `unknown` on every screen that implies action (Ship tag, Merging row, Goal-page callout/merge-gate, Settings verdict); "Global write → DRY-RUN" / "Arm REAL" **disabled + note** ("global FKST_GITHUB_WRITE is deploy-time; no API to read/change it in v1; applied via session restart"). | F25, F16 | patch | Yes | none (posture gap) | Never asserts `REAL`; the real off-switches are global DRY-RUN (no endpoint → disabled) or a GitHub mutation that removes the work (close PR / remove label). | Posture reads `unknown` consistently; elevation controls disabled with the exact note; no fictional success. |

### Milestone M6 — Host-agent / advanced (DEFERRED)

| ID | Title | Issue | Scope / deliverable | Dep | Bump | Plan? | Plane | Honesty | Acceptance |
|---|---|---|---|---|---|---|---|---|---|
| **F27** | *(deferred)* Activity (folded Runs) + redb/codex panels | *Feature:* "Build the Goals Activity view and the Goal-page redb/codex diagnostic panels once a host-agent or backend runs/journal read endpoint exists." | Activity runs table + diagnostic vitals; Deliveries·redb / Runs·codex panels — **only** if a host-agent or backend journal/runs read endpoint lands (file upstream first). | F24, (upstream backend issue) | minor | No | host-agent plane (deferred) | Until then these render the M3/M4 honest "host telemetry not connected" / `unknown`; **no requeue / replay-DLQ / run-mutation controls ever**; in-DLQ is the canonical `unknown`-not-`0`. | Not in v1 acceptance. When built: rows reconstructed-not-ledger; no mutating controls; in-DLQ reads `unknown`. |

---

## D. Screen → phase → data-deps → status

| Screen | Mockup | Increment(s) | Milestone | Primary data deps | v1 status |
|---|---|---|---|---|---|
| **App shell / topbar / nav** | (all) | F0a, F0b, F2 | M0–M1 | none | Built |
| **Packages** (list/detail/create) | `packages.html` | F11, F13 | M3 | `GET/POST /packages`, `GET /packages/:name` | **Truthful v1** (backend-served) |
| **Packages — topology + tri-panel** | `packages.html` | F12 | M3 | `files[]` (FE-derived) | Partial — FE-derived; unparsed→`unknown` |
| **Packages — enable toggle + Apply-changes** | `packages.html` | F14 | M3 | `sessions` cycle | Partial — toggle = intent; Apply = real flow |
| **Settings — hosted-engine pane + Stop** | `settings.html` | F15 | M3 | `health`, `sessions` | **Truthful v1** (backend-served) |
| **Settings — posture/knobs/identity** | `settings.html` | F16 | M3 | none (gaps) | Gap — disabled + note (`unknown`) |
| **New-goal modal** | `overview.html`/`goals.html`/`goal.html` | F17 (graph) → F25 (create) | M3→M5 | `packages` (graph) + GitHub (create) | Graph truthful; create gap until NyxID |
| **GitHub-plane shells** | `overview.html`/`goals.html`/`goal.html` | F18 | M3 | none (honest empty) | Gap — skeleton/empty/disabled |
| **Goals — Issues** | `goals.html` | F21 | M4 | GitHub issues + markers | Gap until NyxID |
| **Overview — Pipeline** | `overview.html` | F22 | M4 | GitHub-derived goal set + vitals | Gap until NyxID |
| **Overview — Board + Needs-you** | `overview.html` | F23 | M4 | GitHub-derived goal set | Gap until NyxID |
| **Goal page / Issue modal** | `goal.html` | F24 | M4 | GitHub markers/PR (redb/codex deferred) | Gap until NyxID; host panels deferred |
| **Actions (create/label/comment/close PR)** | (all) | F25 | M5 | GitHub mutations via proxy | Gap until NyxID |
| **Posture / elevation** | `settings.html`/`overview.html`/`goal.html` | F26 | M5 | none (no posture endpoint) | Gap — `unknown`, disabled |
| **Goals — Activity (folded Runs)** | `runs.html`→`goals?view=activity` | F27 | M6 | host-agent (redb/logs) | **Deferred** — "host telemetry not connected" |
| **Inbox** | `inbox.html` | — | — | GitHub + posture | **Deferred** — not in nav, not a route; ships as Overview Needs-you (F23) |

---

## E. Definition of done

### Per-milestone DoD

- **M0:** `frontend/package.json` exists from **F0a** and arms `frontend-ci.yml`; the gate
  is ordered so **F1 lands immediately after F0a** and its own PR **runs (not no-ops)** the
  full `install→lint→typecheck→build→test` against the F0a app — green because F0a ships at
  least a `passWithNoTests`/trivial passing test; the app shell (F0b, topbar/nav +
  condense-on-scroll hysteresis 140/40) lands **after** the armed gate so its CI exercises it;
  CI green throughout; **rust/docker CI behavior unchanged**; `/inbox` and `/runs` are not
  first-class routes (`/runs`→`/goals?view=activity`).
- **M1:** Every locked primitive renders in all four states (loading/empty/error/`unknown`);
  jsx-a11y + anti-slop lint green; status-never-hue-alone and `unknown`-not-`0` are
  lint-enforced; axe passes the gallery.
- **M2:** All backend hooks typed against `SessionView`/`PackageResponse`; `truth/` unit
  tests green (trust, version tuple + tie-breaks, stage buckets, terminality, `unknown`-not-`0`);
  session polling stops on terminal; `health.version` labeled "backend build".
- **M3:** Packages list/detail/create work against a running backend (409/400 inline);
  Apply-changes stop→poll→create works; conformance is session-derived; Settings engine
  pane + per-package Stop work; every GitHub-plane screen renders a truthful disabled/empty
  state; the **backend e2e happy path stays green**.
- **M4:** Sign-in works; GitHub-plane screens render real poll-derived data with per-source
  freshness; FE never holds a raw GitHub token; GitHub chip FE-tracked (never from `/health`).
- **M5:** Every action maps to exactly one real capability; **no forbidden control exists
  anywhere**; posture never asserts `REAL`; elevation disabled-with-note.
- **M6:** Deferred; tracked as upstream backend issues; no fabricated rows or mutating
  controls if ever built.

### Overall v1 acceptance checklist

- [ ] Issue-first → branch → PR `Closes #N` → changeset → auto-merge-on-green followed for
      every increment; commits small/self-contained; **no `Co-Authored-By`/bot trailer**;
      user identity throughout.
- [ ] `frontend-ci.yml` gates by `frontend/package.json` existence, skips on
      `release-automation`, and is green; rust/docker CI unchanged.
- [ ] Backend-served surfaces (Packages list/detail/create, session cycle, Settings engine
      pane + per-package Stop, New-goal package graph) are **truthful** against a running backend.
- [ ] Every v1 gap renders a **disabled control + honest note** — never fictional success
      (NyxID identity/actions, posture read/write, config knobs, run-journal deep link,
      host-agent Activity/redb/codex, deployment-wide suspend, multi-deployment scope).
- [ ] **`unknown`, never `0`** for any unreachable deciding source; posture reads `unknown`;
      in-DLQ reads `unknown`; a real `0` only on a genuinely successful empty poll.
- [ ] Status is **never hue-alone** (color + text + shape + position); **amber is brand-only**;
      WCAG AA on the dark canvas; colorblind-sim assertions pass.
- [ ] **Responsive/overflow:** `document.documentElement.scrollWidth ≤ clientWidth` at key
      breakpoints up to the 1440 cap on every route; grids use `minmax(0,1fr)` + `min-width:0`.
- [ ] **No fake-live:** poll-derived everywhere with per-source as-of chips; `prefers-reduced-motion`
      honored; no marching dashes / glows / orbiting logo / decorative gradients.
- [ ] **Forbidden actions absent:** no requeue/replay-DLQ, no resume/reopen/retry on terminal
      goals, no per-goal pause, no deployment-wide suspend, no "connect a local host agent".
- [ ] The **backend e2e happy path (`backend/tests`) stays green**; Playwright drives the real
      Packages screen + Settings engine pane + per-package Stop; GitHub/host-agent e2e assert
      the honest disabled/empty states until those planes land.

---

## F. Sequencing notes & what is explicitly deferred

**Why this order.** The scaffold is split so the CI gate arms *before* the first non-trivial
UI component lands: **F0a** ships the toolchain + the `frontend/package.json` gate file + an
empty router/provider and a green `vitest run` (`passWithNoTests` or one trivial test);
**F1** adds `frontend-ci.yml` immediately after — and because F0a already added the gate file,
F1's *own* PR takes the **else-branch and actually runs** `install/lint/typecheck/build/test`
(it does **not** no-op), so F0a must be green for F1 to merge; **F0b** then adds the topbar/nav
shell with condense-on-scroll (a tested hysteresis behavior, DESIGN.md §61) **behind the armed
gate**, so the largest M0 row is exercised by its own CI rather than merging with the workflow
not yet running. No other increment can merge green without F0a + F1, and the
`frontend/package.json` gate is what *arms* `frontend-ci.yml`. Design-system primitives (M1)
precede every screen so screens compose
locked components rather than re-inventing them. The data layer + `truth/` core (M2) precede
the screens because the honesty model (trust, version-ordering, `unknown`-not-`0`) is the
load-bearing logic every surface depends on — and it is pure and unit-testable in isolation.
M3 deliberately ships **only what the real backend serves** plus honest shells for everything
else, so v1 is demonstrably truthful before any GitHub plumbing exists. M4/M5 light up the
GitHub plane and its actions **after** NyxID is confirmed. M6 stays deferred.

**Parallelization.** Within M1, F3/F4/F5 can run in parallel once F2 lands. Within M2, F7/F8
can parallelize after F6; F9 follows F8; F10 follows F6. In M3, F12/F13/F14 parallelize after
F11; F16/F17 after their list/shell deps. In M4, F22/F23/F24 parallelize after F20+F21's
seam. Keep each as its own PR even when parallel.

**Explicitly deferred (with rationale):**

- **Inbox** — the spec is retained (`inbox.html`) but it is **not in nav and not a registered
  route**. Its triage queue ships as the Overview **Needs-you** band (F23). Minting a route
  would re-establish a retired identity; if a placeholder is ever wanted it redirects to
  Overview Needs-you with a plain "unbuilt" note. *Rationale:* DESIGN.md/TRD mark Inbox
  hidden/complex; Needs-you is the v1 stand-in.
- **Standalone Runs** — superseded; folded into Goals as the **Activity** view
  (`/runs`→`/goals?view=activity` redirect only). *Rationale:* TRD folds Runs into Goals; a
  real Runs route would re-establish the retired Runs identity.
- **Host-agent / redb / codex views (F27)** — the redb delivery ledger/DLQ, framework/codex
  logs, and runtime topology are **not reachable** via NyxID or the hosted v1 API; the engine
  is **hosted, not local**, so "connect a local host agent" is forbidden. They render
  "host telemetry not connected" / `unknown` until a host-agent or backend runs/journal read
  endpoint lands (file upstream first). *Rationale:* the plane is optional/deferred in the brief.
- **NyxID-gated actions until NyxID lands** — all GitHub reads/writes (Goals Issues, Overview
  canvas, Goal lifecycle, create-issue/enable, label/comment/close-PR, avatar identity,
  connected repos) are behind the `api-github` proxy that is **not integrated in v1**. They
  ship as disabled-with-note (M3 shells), then light up in M4/M5. *Rationale:* NyxID is a v1
  gap; the layout/copy is built now, the wiring is deferred and **gated on the §F confirmations**
  (broker scope, proxy CORS for the FE origin, `@nyxids/oauth-react` availability + version pin).
- **Posture write (and read) until an endpoint exists** — `FKST_GITHUB_WRITE` is deploy-time
  env with **no read or write route**; posture reads `unknown` and the REAL elevation flow ships
  disabled-with-note (F26). *Rationale:* no posture endpoint in v1; the FE must not assert `REAL`
  or fake an elevation. Filing a posture/config-read endpoint upstream is the correct path —
  not faking it here.

**Backend dependencies the FE must file upstream (not work around):**

- Tighten CORS from `allow_origin(Any)` to the real FE origin before production (a documented
  dev-only TODO in the backend `router.rs`).
- A posture/config-read endpoint (so Settings can show real `FKST_GITHUB_WRITE` and knobs
  instead of `unknown`).
- A runs/journal-read endpoint exposing `run_key`/journal coordinates (so the "Open run journal
  on GitHub" deep link can resolve instead of staying disabled).

Each is a backend issue in *this* repo; the FE renders the gap honestly until it lands.
