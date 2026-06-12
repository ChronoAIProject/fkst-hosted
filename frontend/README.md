# frontend

The React SPA for **fkst-hosted** — a mission-control console for developers running
autonomous, GitHub-issue-driven AI dev loops over the hosted engine. It fronts this
repo's Rust/Axum [`backend/`](../backend) (packages + sessions + health), reads GitHub
through **NyxID** (no token custody in the browser), and is a **read-mostly observer** —
never a second source of truth.

> **Status: planning (docs-first).** This folder currently holds the **design system, the
> reference mockups, the architecture brief, and the implementation plan** — *no application
> code yet, by design.* Code lands only after the brief + plan are approved, then one
> PR-sized increment at a time per the repo's git workflow (issue → branch → PR `Closes #N`
> → changeset → auto-merge on green CI). See [`../CLAUDE.md`](../CLAUDE.md).

## Layout

```
frontend/
├── README.md                     # you are here
├── docs/
│   ├── design.md                 # the LOCKED design system (tokens, type, components, anti-slop)
│   ├── ARCHITECTURE.md           # architecture brief — stack, planes, data/truth model, IA, build integration
│   └── IMPLEMENTATION-PLAN.md    # PR-by-PR roadmap, milestones, dependency ordering
└── mockups/                      # the 7 locked production screens (self-contained HTML; open in a browser)
    ├── overview.html  goals.html  packages.html  goal.html
    ├── runs.html      settings.html
    └── inbox.html                # deferred (kept for reference; not in the v1 nav)
```

## What it fronts (the real v1 surface)

- **Hosted backend (this repo):** `GET /api/v1/health` · `GET|POST /api/v1/packages`
  (create-only, 409 on dup) · `POST /api/v1/sessions` (one live session/package) ·
  `GET /api/v1/sessions/:id` (`pending→validating→running→stopping→stopped/failed`) ·
  `POST /api/v1/sessions/:id/stop`. Data is **poll-derived (~5-min cron), not live.**
- **GitHub plane (via NyxID `api-github` proxy):** repos, issues, PRs, trusted `state:v1`
  comment markers, labels. *Not integrated yet — a v1 gap.*
- **Host-agent plane (optional, read-only):** redb delivery ledger / logs / topology.
  *Deferred.*

## Information architecture (locked)

Primary nav: **Overview · Goals · Packages.** Settings opens from the **avatar**.
Goal detail is an **Issue modal** (from any goal) and a full **Goal page**. **Runs** folds
into Goals. **Inbox is deferred** (hidden from nav). Overview has two views — **Pipeline**
(control-room hero) and **Board** (kanban).

## Provenance

- **Design system** — verbatim copy of the locked `DESIGN.md` (FKST Mission Control, v2)
  from `fe-blueprint/`. Treat [`docs/design.md`](docs/design.md) as authoritative; the
  upstream blueprint (`00-FRONTEND-TRD.md`, `01-DATA-REFERENCE.md`) is the deeper reference.
- **Mockups** — copied from the locked screen set `designs/goal-board-20260611/`. They are
  **fidelity targets**, not shipped code; all seven share one token system and cross-link by
  relative path, so they browse as a set.

## Honesty contract (non-negotiable)

Poll-derived, not live — say so. An unreachable source reads **"unknown", never "0"**.
v1 gaps render as a **disabled control + an honest note**, never fictional success. Status
is never hue-alone. Amber is brand-only, never a status. Every action maps to exactly one
real capability (a GitHub mutation, a substrate re-trigger, or a posture/topology change) —
nothing fabricated.
