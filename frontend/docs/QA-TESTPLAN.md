# FKST Console — QA Test Plan (v1-basic)

Manual + automated QA for the FKST frontend. Covers every flow the **hosted v1 API** serves, every **honest-gap** surface (GitHub plane / NyxID cut from v1), and the cross-cutting invariants (honesty, responsive, a11y, anti-slop). Grounded in the 2026-06-15 verification (`./VERIFY-REPORT.md`): the **API contract** is direct-verified (repeatable) and the **UI** is verified at **smoke level** (live-data render + a proven UI→backend write + honest gaps). The cases below marked beyond that — topology detail, Apply-changes/Settings-Stop, 400/409 inline mapping, Board, empty-vs-unreachable, a11y depth — are **manual sign-off** until the UI E2E harness is strengthened (per the codex audit in VERIFY-REPORT).

**Legend:** each case has **Steps** → **Expected** → record **PASS/FAIL**. "Honesty rules" are release-blocking: unreachable → `unknown` (never `0`); v1 gaps render disabled + an honest note; amber is brand-only (never a status); nothing fakes "live".

---

## A. Data provenance — what is LIVE API data vs hardcoded

So a reviewer can tell a real-backend showcase from static chrome. **On a live route pointed at the hosted backend:**

- 🟢 **LIVE** — real data from the hosted v1 API; changes when the backend changes.
- 🔵 **FE-DERIVED** — computed by the FE *from* live API data (no dedicated endpoint); changes with the package data.
- 🟡 **SEED** — hardcoded placeholder/example shipped in the app; identical every load; NOT from any API.
- ⚪ **GAP** — honest `unknown`/disabled; no data plane in v1 (lights up post-NyxID / when an endpoint lands).
- 🟣 **MOCK** — populated fixtures that exist **only** in Storybook `Mock /` pages; never on a live route.

| Surface / element | Source | Notes |
|---|---|---|
| Topbar — logo, primary nav | 🟡 SEED | static chrome |
| Topbar — GitHub freshness chip (`github — unknown`) | 🟡 SEED | hardcoded literal; **never** fed by `/health` (by design) |
| Topbar — avatar / "Sign-in pending" | ⚪ GAP | NyxID not wired |
| **Packages** — list (names, flat/composed badge, deps chips) | 🟢 LIVE | `GET /packages` + `GET /packages/:name` |
| **Packages** — intro lede + Company·Dept·Person levels | 🟡 SEED | static explanatory copy |
| **Packages** — topology (SOURCES band, department rows) | 🔵 FE-DERIVED | parsed from the package's `files[].path` — there is **no topology endpoint** |
| **Packages** — queue wiring / codex tags / conformance / role / dept counts / namespace / "graph …" | ⚪ GAP | render `unknown` + "not parsed / not exposed by the v1 API" |
| **Packages** — enable toggle | 🟡 SEED (intent) | local UI intent only (no enable endpoint); "applies via session restart" |
| **Add-package** — form fields | user input | submitted verbatim |
| **Add-package** — result (201 / 409 dup / 400 invalid, inline) | 🟢 LIVE | `POST /packages`; server authoritative |
| **Settings** — engine status, mongo, version "backend build" | 🟢 LIVE | `GET /health` |
| **Settings** — session status + Stop (tab-known sessions only, §1.1) | 🟢 LIVE | `GET/POST /sessions/:id` |
| **Settings** — posture (`FKST_GITHUB_WRITE`/repo/checks/"posture as of") | ⚪ GAP | hardcoded `unknown` — no posture endpoint in v1 |
| **Settings** — deployment knobs, identity, repos, sign-out | ⚪ GAP | disabled + grounding notes |
| **Settings** — poll-interval note ("interval = 5m declared statically") | 🟡 SEED | static fact, not a setter |
| **New-goal** — package graph | 🟢 LIVE | `GET /packages` + `composed_deps` |
| **New-goal** — repository dropdown (`example-org/repo-a…c — example`) | 🟡 SEED | three hardcoded EXAMPLE repos — **not** live GitHub repos |
| **New-goal** — title/description fields | user input | |
| **New-goal** — "Create issue & enable" submit | ⚪ GAP | disabled "requires NyxID sign-in" |
| **Overview** — Pipeline/Board, vitals, Needs-you | ⚪ GAP (live) / 🟣 MOCK | live routes pass **no data** → `—`/`unknown` + "no GitHub plane connected"; the populated look exists only as Storybook mock |
| **Goals** — Issues table / Activity | ⚪ GAP (live) / 🟣 MOCK | empty shell + "host telemetry not connected" live; populated = mock |
| **Goal page** (`/goals/:id`) — decision/timeline/merge-gate/diagnostics | ⚪ GAP (live) / 🟣 MOCK | skeleton/empty live; populated = mock |
| Storybook `Mock /` Overview/Goals/Goal pages | 🟣 MOCK | fixtures transcribed from the locked mockups; carry a visible MOCK-DATA banner |

**Showcase summary:** the screens that render **LIVE backend data** are **Packages** (list/detail/create + FE-derived topology), **Settings → hosted engine** (health + tab-known sessions), and the **New-goal package graph**. Every GitHub-plane screen (Overview/Goals/Goal) is an **honest empty shell** on live routes and is only *populated* in Storybook mocks. The **GitHub chip** and the **New-goal repo dropdown** are **static placeholders**, not live. When demoing "the APIs", drive **Packages + Settings + the New-goal graph**.

---

## 0. Environment setup

### 0.1 Automated suites (no backend needed)
From `frontend/`:
```
npm ci
npm run lint          # eslint --max-warnings 0 (anti-slop rules)
npm run typecheck     # tsc --noEmit
npm run test          # vitest (unit + truth/ + hooks via MSW)
npm run build         # vite production build
npm run build-storybook
npx playwright test   # e2e smoke: honest-empty shells + overflow 480/780/980/1440 + console errors
```
All must be green. Storybook (`npm run storybook`) hosts the component gallery + the `Mock /` pages.

### 0.2 Local throwaway backend (for the backend-served flows in §1)
Docker path: `docker compose up mongo` (backend/) then run the API. Native path (no Docker, as used in verification):
1. Engine: build `fkst-substrate` @ the ref in `backend/engine.ref` → `target/release/fkst-framework`.
2. Mongo: any local `mongod` on `:27017` (throwaway dbpath).
3. API: run `fkst-hosted-api` with `MONGODB_URI=mongodb://127.0.0.1:27017`, `FKST_HOSTED_PORT=8080`, `FKST_HOSTED_ENGINE_FRAMEWORK_BIN=<built engine>`, `FKST_JOURNAL_GITHUB_ENABLED=false`. Confirm `curl :8080/health` → `{status:ok,mongo:up}`.
4. FE: `VITE_FKST_API_BASE=http://127.0.0.1:8080 npm run build && npm run preview`.
> The package store supports create, update, and delete via the backend API — only ever point write tests at a throwaway backend, never a shared deployment.

---

## 1. Backend-served flows (real data, requires §0.2)

### TC-1.1 Packages list + detail
- **Steps:** open `/packages` against a backend with ≥1 package.
- **Expected:** each package row shows mono name + flat/composed badge (composed = `composed_deps.length>0`, amber-tinted) + deps chips. Intro lede + Company·Department·Person levels present. Fields the API doesn't expose (conformance, role, dept counts) render `unknown` with one honest note — never fabricated numbers. **UI currently does not have edit/delete controls (update/delete available via API; UI coming soon).**

### TC-1.2 Empty vs unreachable (the honesty split)
- **Steps:** (a) point at an empty store; (b) stop the API and reload.
- **Expected:** (a) genuine empty state ("no packages…"); (b) "package store unreachable — unknown" — **the two must look/read differently**; unreachable must never render as an empty list or `0`.

### TC-1.3 Add package — server-authoritative
- **Steps:** "+ Add package" → submit a **new** valid package (name `^[A-Za-z0-9_-]+$`, ≥1 file incl. an engine entry `departments/*/main.lua` or `raisers/*.lua`).
- **Expected:** 201 → list refreshes → toast "Created — composes on next session start" (not "deployed/live"). The modal carries the 409/conformance-at-start note. Files contain **user-supplied content** (no stubbed/auto-filled content sent).

### TC-1.4 Add package — duplicate → 409 inline
- **Steps:** submit an existing name.
- **Expected:** inline error "name already exists (a revision is a new name)". No success, no toast.

### TC-1.5 Add package — invalid → 400 inline
- **Steps:** submit bad name (e.g. `bad name!`) or no engine entry.
- **Expected:** field-level inline error from the server; client pre-validation may also flag it. Server is authoritative.

### TC-1.6 Topology (derived from `files[]`)
- **Steps:** view a package with `departments/<d>/main.lua` and/or `raisers/<n>.lua`.
- **Expected:** SOURCES band lists raisers (cadence `—` "declared in Lua, not parsed"); department rows derived from paths; queue wiring/codex tags render `unknown` with the "not parsed by this console" note. Caption "derived from file paths · scanned at startup". Read/write tri-panel present (source read-only · FE manages set/topology/posture via restart · writes→GitHub under REAL).

### TC-1.7 Apply-changes session cycle (§1.1 registry contract)
- **Steps:** with a session this tab created, click "Apply changes" for that package.
- **Expected:** three-phase progress NAMING the package: `<pkg> · stop requested (202 ack) → waiting for stopped → starting new session`. Uses stop→poll(GET)→create. With NO tab-known session: button disabled + the §1.1 gap copy ("current session id not exposed by the v1 API — this console can only manage sessions it started this tab"). On 409 after a successful stop: "session stopped, but restart failed — package already has a live session…".

### TC-1.8 Settings — engine pane
- **Steps:** open `/settings` (from the avatar, not nav).
- **Expected:** engine status from live `/health` — healthy (green dot) / degraded (typed 503) / unreachable→`unknown` (never "live", never `0`); `version` labeled **"backend build"**. Mongo up/down as text+dot.

### TC-1.9 Settings — per-package Stop
- **Steps:** for a tab-known session, click Stop → confirm dialog → proceed.
- **Expected:** danger-outline confirm; on proceed → 202 ack copy ("requests a stop (202 ack); the console polls until stopped/failed"), polling continues to terminal. Stop failure → inline red note, dialog stays mounted. Stale 404 → "session no longer found — stale registry entry" + entry cleared. **No deployment-wide suspend** anywhere.

### TC-1.10 New-goal modal — package graph (backend-served part)
- **Steps:** "+ New goal" → inspect the Packages section.
- **Expected:** read-only graph from `GET /packages` + per-package `composed_deps`, with the "deployment-wide, applies on restart — not per-goal" note. Unreachable → `unknown` (not empty-as-zero).

---

## 2. Honest-gap flows (no GitHub plane in v1 — must render honestly)

### TC-2.1 Overview shells (Pipeline + Board)
- **Expected:** Pipeline rail reads as a **pipe** (slim Intake/Merged end-caps, continuous conduit line, 4 stage columns, never wraps to 2 rows — scrolls then stacks). Counts render `—`/`unknown`, **never `0`** on first paint. Board toggle (amber **stroke** active, not fill). One honest "no GitHub plane connected — sign-in pending" line. **Needs-you** = disabled band "Needs-you unavailable — requires GitHub plane (NyxID) integration" (the string "Nothing needs you" must **not** appear on a live route — mock stories only). Vitals = one contained panel, all `unknown`.

### TC-2.2 Goals shell
- **Expected:** Issues table chrome + honest empty; Issues/Activity toggle; Activity = "host telemetry not connected". `/runs` redirects to `/goals?view=activity`. Filter controls disabled with accessible names.

### TC-2.3 Goal page shell (`/goals/:id`)
- **Expected:** decision header, lifecycle timeline rail, diagnostics rail — skeleton/empty; merge gate "not at the gate yet"; redb/codex panels "host telemetry not connected".

### TC-2.4 Posture (everywhere)
- **Expected:** posture reads `posture unknown (deploy-time)` on every action-implying surface; "Arm REAL"/elevation disabled + grounding note. **Never** asserts REAL or DRY-RUN as the current state.

### TC-2.5 New-goal submit (gap)
- **Expected:** "Create issue & enable" disabled + "requires NyxID sign-in"; honest "next ~5-min poll → Design" footnote. Opens from topbar (condensed state), Overview, and Goals.

### TC-2.6 GitHub freshness chip
- **Expected:** static neutral "github — unknown"; **never** fed by backend `/health`; no green/live indication.

---

## 3. Storybook mock pages (the populated showcase)

### TC-3.1 `Mock /` pages render
- **Steps:** open Storybook → `Mock / Overview`, `Mock / Goals`, `Mock / Goal`.
- **Expected:** populated per the locked mockups (stage goals, gold pressure rule, **red "REAL · #152 → integration"** Ship tag, vitals incl. string values; Goals issue rows; Goal lifecycle timeline incl. the "stale replay skipped · CAS skip-stale" event, current node amber, merge-gate posture `unknown`). Each carries a visible **MOCK DATA** banner. `Mock /` data must not leak to any live route.

### TC-3.2 Component states gallery
- **Expected:** primitives + status vocabulary render in loading/empty/error/`unknown`; `Packages / States` + `Settings / States` show each named state (Loading=skeletons, GenuineEmpty ≠ UnreachableUnknown, Degraded, NoKnownSessionGap).

---

## 4. Cross-cutting invariants (every screen)

### TC-4.1 Responsive / overflow
- **Steps:** at 1440 / 980 / 780 / 480 widths (Playwright covers this).
- **Expected:** `document.documentElement.scrollWidth ≤ window.innerWidth` on every route (no horizontal scrollbar). Pipeline scrolls→stacks, never a 2-row grid. 1440 hard cap, centered.

### TC-4.2 Accessibility
- **Expected:** keyboard reachable; `:focus-visible` amber ring on every interactive element; icon-only buttons + disabled gap controls have accessible names; modals trap focus + Esc/backdrop close; nav landmarks + heading structure; status never hue-alone (color + text + shape).

### TC-4.3 Console + motion + brand
- **Expected:** **zero console errors** on every route; `prefers-reduced-motion` honored; no marching dashes / glows / orbiting logo / decorative gradients / `system-ui` type; amber appears only on brand/active-segment/focus, never as a status.

### TC-4.4 Topbar condense-on-scroll
- **Expected:** condenses past 140px, expands under 40px (hysteresis, no flicker); topbar "+ New goal" appears only when condensed (no duplicate primary with the toolbar one).

---

## 5. Forbidden actions (must be ABSENT everywhere)
Confirm none of these exist in the UI (action-grounding contract):
- ❌ requeue / replay-DLQ
- ❌ resume / reopen / retry on a terminal goal (blocked/impl-failed/merged) — re-engagement is only "New issue from this"
- ❌ per-goal pause
- ❌ deployment-wide suspend (only per-package session stop)
- ❌ "connect a local host agent" (engine is hosted)

---

## 6. Sign-off
- [ ] §0 automated suites green (lint/typecheck/test/build/build-storybook/playwright)
- [ ] §1 backend-served flows pass against a throwaway backend
- [ ] §2 honest-gap flows pass (no `0`-for-unknown, no asserted posture, no fake-live)
- [ ] §3 Storybook mock pages + state gallery render
- [ ] §4 responsive/a11y/console/condense invariants hold
- [ ] §5 no forbidden actions present

QA: ____________  Date: __________  Build/commit: __________
