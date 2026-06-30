# CLAUDE.md

This file guides Claude Code (and any AI agent) when working in the **fkst-hosted** repository. These instructions are authoritative for this repo and must be followed exactly.

## Project Overview

**fkst-hosted** serves the **fkst** project's hosting-related concerns and is deployed as **ChronoAI's cloud services**.

- **Backend:** Rust-based backend service.
- **Frontend:** React.
- **Purpose:** User-facing and public interfaces for the fkst project, running as ChronoAI's hosted cloud offering.

## Scope & Boundaries

fkst-hosted has a deliberately narrow scope. Respect these boundaries on every change:

- ✅ **In scope:** Only user-facing and public interfaces that matter to the user.
- ❌ **Out of scope:** Anything related to the **kernel engine**. fkst-hosted does **not** change or include kernel-engine code.

> When a task seems to require touching engine internals, stop and reconsider — that work belongs upstream (see below), not in this repo.

## Upstream Source Repositories

These are **reference-only** dependencies. Do **not** modify them from within fkst-hosted; consult them to understand contracts and behavior.

| Component | Repository |
|-----------|------------|
| Engine    | https://github.com/ChronoAIProject/fkst-substrate |
| Packages  | https://github.com/ChronoAIProject/fkst-packages   |

## Integrations & Platform

fkst-hosted integrates with the following ChronoAI platform services. When doing related work, **always reference the latest `main` branch** of the corresponding repo for the current contracts and APIs.

| Integration | Area | Reference (latest `main`) |
|-------------|------|---------------------------|
| **NyxID** | IAM (identity & access management). fkst-hosted is deployed **under NyxID as one of its downstream services**. | https://github.com/ChronoAIProject/NyxID |
| **Ornn** | Agent-skill features. | https://github.com/ChronoAIProject/Ornn |

- For any **NyxID / IAM**-related work, reference NyxID's latest `main`.
- For any **Ornn / agent-skill**-related work, reference Ornn's latest `main`.

## Repository Layout

| Area      | Stack | Responsibility |
|-----------|-------|----------------|
| Backend   | Rust  | Hosted backend service, public APIs, user-facing endpoints |
| Frontend  | React | User-facing web interface |

## API Contract (OpenAPI)

The control plane (`backend/`, a single Rust crate) serves a **dynamically generated OpenAPI 3.1 document at `GET /openapi.json`**. It is assembled at runtime from the live Axum routes and Rust types via `utoipa` + `utoipa-axum` — there is **no static / checked-in spec file**, and the route registration *is* the documented path (`utoipa-axum`'s `OpenApiRouter` + `routes!`), so the spec never drifts from the code. The assembly + serving lives in `src/openapi.rs`; `src/router.rs::build_router` composes the routers and `split_for_parts()` yields `(Router, OpenApi)`.

When you add or change a **public** HTTP endpoint, the spec does **not** auto-reflect the handler signature — you must keep it in sync:

- **Annotate the handler** with `#[utoipa::path(method, path = "/x/{id}", tag, operation_id, params(...), request_body = ..., responses(...))]`. The `path` here is the single source of truth (`utoipa-axum` maps `{id}` → axum's `:id`). A handler without this annotation will NOT appear in the spec.
- **Register via `OpenApiRouter`**: every `routes::*::router()` returns `utoipa_axum::router::OpenApiRouter<AppState>` and adds routes with `.routes(routes!(handler, ...))` (group same-path handlers in one `routes!`). Do not introduce a bare `axum::Router` for a public route module.
- **Derive schemas**: `#[derive(ToSchema)]` on every request/response DTO; `#[derive(IntoParams)]` + `#[into_params(parameter_in = Query)]` on typed query structs. Error responses reference the public `error::ErrorEnvelope`.
- **Security**: protected `/api/v1/*` operations carry `security(("NyxIdIdentity" = []))`; the public surface (`/health`, `/metrics`, `/openapi.json`, the signature-verified GitHub App webhook) carries none.

Scope and constraints:

- **Wire types** are plain modules in the crate and derive `ToSchema` directly (the backend is one crate — there is no separate shared/worker crate, so no off-by-default `schema` feature to gate). A new request/response DTO needs `#[derive(ToSchema)]`, or it won't appear in the spec.
- **Scope is the public surface only**: `/api/v1/*`, `/health`, `/metrics`, and the GitHub App webhook (only when a webhook secret is configured — the spec tracks live config).
- **Component names** are derived from the Rust type identifier, so duplicate idents collide in the spec — give colliding types distinct names or consolidate them into one type.
- **Version pins**: `utoipa = "5"`, `utoipa-axum = "0.1"` (the axum-0.7 line; `utoipa-axum` 0.2+ targets axum 0.8 — do not bump it until axum itself is upgraded).
- **Keep `tests/openapi.rs` green**: it drives the real `build_router` and asserts the spec's paths/schemas/security.

## Git Workflow

### Commit Rules

- **Every commit must be small and self-contained.** No large commits are allowed.
- Each commit should represent one coherent, reviewable unit of change.

### Commit Authorship & Identity

- **Never include `Co-Authored-By`** — or any other AI / co-author trailer — in commit messages.
- **Always use the user's own GitHub identity** for every git operation (commits) and GitHub operation (issues, PRs, reviews, merges). Never commit or act as a bot, shared, or AI/Claude identity.
- Git is configured with the human maintainer's own name/email and the `gh` CLI is authenticated as that same person — keep the two consistent.

### Branch Model

| Branch         | Role |
|----------------|------|
| `main`         | **Production** branch. |
| `develop`      | **Active development** branch. |
| `develop-auto` | Branch actively developed and evolved by **unattended AI agent looping sessions**. |

### Branching & Merge Rules

- All features and bug fixes **must** land via a **pull request** into `develop` or `develop-auto`.
- **Only `develop` may be merged into `main`.** (`develop-auto` does not merge directly into `main`.)
- **No force push** is allowed on `main`, `develop`, or `develop-auto`.

### Issue & Pull Request Discipline

- **All work must be done via a proper pull request.** No direct commits to shared branches (`main`, `develop`, `develop-auto`); always branch, then open a PR.
- **Every pull request must have a corresponding GitHub issue.** Open the issue first, then reference it from the PR so it auto-closes on merge (e.g., `Closes #123`).
- A PR without a linked issue is not ready to merge.
- Standard flow: **open an issue → create a branch → implement → open a PR linking the issue → review → merge**.

### Auto-merge Policy (AI agents)

- **Unless the user explicitly says otherwise, auto-merge every PR you open into `develop` as soon as CI passes** (all required checks green). Use GitHub auto-merge: `gh pr merge --auto --merge`.
- **If any CI check fails, work on the resolution and auto-merge once CI passes.** Never leave a red PR open or hand it back unresolved.
- Applies to PRs targeting `develop` (the unattended `develop-auto` loop follows the same auto-merge-on-green behavior). PRs into `main` still require review (1 approval); a release is cut manually as a git tag on `main`.

### Flow

```mermaid
graph LR
    I[GitHub issue] --> F[feature / bugfix branch]
    F -->|pull request: Closes #issue| D[develop]
    F -->|pull request: Closes #issue| DA[develop-auto]
    D -->|merge| M[main / production]
```

## Issue & PR Templates

Every issue and pull request uses a standard template, stored under `.github/`:

| Template | Path | Use |
|----------|------|-----|
| Bug report | `.github/ISSUE_TEMPLATE/bug_report.md` | Report a defect in a user-facing/public interface. |
| Feature request | `.github/ISSUE_TEMPLATE/feature_request.md` | Propose a new user-facing feature or improvement. |
| Issue chooser config | `.github/ISSUE_TEMPLATE/config.yml` | Disables blank issues; routes engine/packages issues upstream. |
| Pull request | `.github/PULL_REQUEST_TEMPLATE.md` | Auto-applied to every PR; requires a linked issue. |

- GitHub auto-applies these templates when opening issues/PRs in the web UI.
- When creating issues/PRs via `gh` or the API (including unattended AI agent loops), fill the same template fields so structure and the required issue link are preserved.

## Versioning

The product version lives in the root `package.json` (`version`) and is read by
the Docker build. There is **no automated release pipeline** (the Changesets +
release-note + tag workflows were removed): PRs into `develop` do **not** need a
changeset, and a release — if ever cut — is a plain git tag on `main`.

## CI (pull requests into `develop`)

PRs into `develop` run exactly five checks, all under `.github/workflows/`:

| Check | Workflow | What it does |
|-------|----------|--------------|
| `rust lint` | `rust-ci.yml` | `cargo fmt --check` + `cargo clippy --all-targets -D warnings` |
| `rust build` | `rust-ci.yml` | `cargo build --workspace --locked` |
| `rust test` | `rust-ci.yml` | `cargo test --workspace --locked` |
| `docker build` | `docker-build.yml` | builds `backend/Dockerfile` `--target server-builder` |
| `gitleaks` | `gitleaks.yml` | scans the working tree for committed secrets |

Keep this set minimal — do not add new PR gates without good reason.

## Quick Rules Summary

- Stay within the user-facing/public-interface scope; never touch the kernel engine.
- The control plane serves a dynamic OpenAPI 3 spec at `/openapi.json` (no static file). New/changed public endpoints MUST be annotated with `#[utoipa::path]` + `ToSchema`/`IntoParams` and registered via `OpenApiRouter`/`routes!`; pin `utoipa-axum` to `0.1` (axum 0.7). See **API Contract (OpenAPI)**.
- The fkst deployables run exclusively on Kubernetes (per-deployable `k8s_sample/` dirs); `docker-compose` is not used in this repo.
- Treat the upstream engine and packages repos as read-only references.
- Keep commits small and self-contained.
- Never add `Co-Authored-By`; always act under the user's own GitHub identity (never a bot/AI identity).
- All work goes through a pull request — no direct commits to shared branches.
- For PRs into `develop`, auto-merge as soon as CI is green; if CI fails, fix it then auto-merge — unless told otherwise.
- Every PR must have a corresponding GitHub issue and link it (`Closes #N`).
- Use the issue/PR templates under `.github/`.
- Use pull requests into `develop` or `develop-auto`; only `develop` merges into `main`.
- Never force push `main`, `develop`, or `develop-auto`.
- For NyxID / IAM work, reference NyxID's latest `main`; for Ornn / agent-skill work, reference Ornn's latest `main`.
- PRs into `develop` run exactly five checks (rust lint/build/test, docker build, gitleaks); there is no changeset or release-note requirement.
- The product version lives in root `package.json`; there is no automated release pipeline — releases are manual git tags on `main`.
