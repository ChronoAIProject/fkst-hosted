# fkst-hosted

**fkst-hosted** is ChronoAI's hosted control plane and web experience for
running **fkst** coding sessions against GitHub repositories. Users declare
sessions and work through GitHub issues; the control plane reconciles that
declared state, manages the session runtime, and writes progress and pull
requests back to GitHub.

## Current capabilities

- **Run issue-driven sessions.** Install the ChronoAI GitHub App, open a trigger
  issue, and queue isolated tasks as issues carrying the session's work label.
- **Operate sessions from the dashboard.** Sign in with GitHub to browse
  installations and repositories, create or stop sessions, add work items, and
  inspect session status and outcomes.
- **Inspect session activity.** Authorized users can browse redacted logs,
  historical runs, and the live engine observation read model.
- **Manage reusable environments.** Create, update, and remove named environment
  profiles. Install commands are validated in an isolated pod before a profile
  is persisted.
- **Manage repository access.** Connect existing repositories, create new ones,
  and open the appropriate GitHub installation settings from the dashboard.

Session configuration is frozen after registration. Closing the trigger issue
permanently retires the session; the dashboard's **Stop** action performs that
same GitHub lifecycle operation.

## Get started

The web application provides three user-facing entry points:

- `/` — product overview
- `/get-started` — GitHub App installation, trigger format, work queue, status,
  logs, and lifecycle guide
- `/dashboard` — authenticated repository and session operations

For the complete issue contract, configuration grammar, authorization rules,
and operational behavior, see the
[`fkst-control-plane-manual`](skills/fkst-control-plane-manual/SKILL.md).

## API

The control plane serves a live **OpenAPI 3.1** document at
`GET /openapi.json`. It is generated from the operations registered by the
running server and is the source of truth for available paths and schemas.
Authentication and authorization are enforced by individual handlers and are
not fully represented as OpenAPI security schemes; use the operator manual and
the web application for the supported user workflows.

## Repository layout

- `backend/` — Rust control plane, GitHub reconciliation, runtime dispatch, and
  HTTP API
- `frontend/` — React web application, user guide, and authenticated dashboard
- `deploy/kubernetes/` — Kubernetes manifests, validation tools, and recovery
  runbooks
- `skills/fkst-control-plane-manual/` — canonical user and operator contract
- `apps/local-qa-runtime/` — independently buildable boundary for **Local QA
  Host** and the reserved hardened Runtime shells

Local QA Host is an activated executable application boundary with a
fail-closed startup contract. It supports no configuration and performs no
runtime capability: every invocation exits with status `1` and writes
`fkst-local-qa-host: no supported configuration` followed by one line-feed byte
to stderr. The launcher, supervisor, guest agent, Secret Broker, and workers
remain inert scaffolds. See
[`apps/local-qa-runtime/README.md`](apps/local-qa-runtime/README.md).

Kernel-engine code remains upstream in `fkst-substrate`, and shared fkst
packages remain upstream in `fkst-packages`; both are reference-only from this
checkout.

## Development and deployment

Run the frontend development server (it proxies `/api` to the control plane on
port `8080`):

```bash
cd frontend
npm ci
npm run dev
```

Use `npm run typecheck`, `npm run lint`, `npm run test`, and `npm run build` for
the frontend's local verification gates.

- The checked-in Kubernetes deployment sources and validation commands are
  documented in [`deploy/kubernetes/README.md`](deploy/kubernetes/README.md).
- The full local stack procedure is in the
  [FKST Local Deployment Guide](CLAUDE.md#fkst-local-deployment-guide).
