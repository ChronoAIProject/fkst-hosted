# fkst-hosted

**fkst-hosted** is ChronoAI's hosted cloud service for the **fkst** project. It
exposes an HTTP API that lets you:

- **Store and manage fkst packages** — the lua bundles the engine runs.
- **Generate a package from a plain-English description** with AI.
- **Share packages** with other users or your organization.
- **Run a package** as an engine **session** and follow it to completion.
- **Define goals** and **trigger** them against a GitHub repository (existing
  or freshly created for you).
- **Work with GitHub issues** across all of your linked GitHub accounts from one
  place.

This README is the user's guide to those APIs. You don't need to know anything
about how the service is built to use it.

---

## Getting started

### Base URL

Every endpoint lives under your fkst-hosted deployment. The examples below use a
shell variable for the base URL — set it to your deployment's address:

```sh
export FKST_API="https://your-fkst-hosted.example.com"
```

All application endpoints are versioned under `/api/v1`.

### Authentication

fkst-hosted runs as a service behind **NyxID**. Every call (except the health
check) requires a **NyxID access token**, sent as a bearer token:

```sh
export TOKEN="<your-nyxid-access-token>"

curl -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/packages"
```

A missing or invalid token returns `401 Unauthorized`. Requesting something you
don't have access to returns `403 Forbidden` (or `404` for other people's
private packages, so their existence isn't leaked).

### Health check (no token needed)

```sh
curl "$FKST_API/health"
# {"status":"ok","mongo":"up","version":"0.0.0"}
```

Returns `200` when the service is healthy, `503` when it is degraded.

### How errors look

Every error uses the same JSON envelope, so you can handle them uniformly:

```json
{ "error": "not_found", "message": "package not found: billing-pipeline" }
```

| Status | `error` | When |
|--------|---------|------|
| `400` | `invalid_request` | Malformed body or invalid name/field |
| `401` | `unauthorized` | Missing or invalid token |
| `403` | `forbidden` | You lack permission |
| `404` | `not_found` | Resource doesn't exist (or isn't visible to you) |
| `409` | `conflict` | Clashes with current state (e.g. duplicate name, busy package) |
| `422` | `unprocessable` | Request understood but can't proceed (e.g. GitHub app not installed) |
| `429` | `rate_limited` | Upstream (GitHub) rate-limited you — see the `Retry-After` header |
| `503` | `unavailable` | A dependency is temporarily unavailable |

---

## Packages

A **package** is the unit fkst runs: a name plus a set of lua files (and an
optional list of composed dependencies). It must contain at least one **engine
entry file**:

- a department entry — `departments/<name>/main.lua`, or
- a raiser entry — `raisers/<name>.lua`

| Method | Path | Does |
|--------|------|------|
| `POST` | `/api/v1/packages` | Create a package from JSON |
| `GET` | `/api/v1/packages` | List package names you can see (add `?filter=shared` for only those shared with you) |
| `GET` | `/api/v1/packages/{name}` | Fetch one package with its files |
| `PUT` | `/api/v1/packages/{name}` | Replace a package's files and dependencies |
| `DELETE` | `/api/v1/packages/{name}` | Delete a package |
| `POST` | `/api/v1/packages/{name}/archive` | Create a package by uploading a `.zip` |
| `PUT` | `/api/v1/packages/{name}/archive` | Replace a package from a `.zip` |
| `POST` | `/api/v1/packages/generate` | Generate a package draft from a description (see [below](#generate-a-package-with-ai)) |

### Create a package

```sh
curl -X POST "$FKST_API/api/v1/packages" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "billing-pipeline",
    "files": [
      { "path": "departments/billing/main.lua", "content": "return {}" }
    ],
    "composed_deps": []
  }'
```

- `name` — letters, digits, underscore and hyphen only (`[A-Za-z0-9_-]+`).
- `files` — a list of `{ "path": ..., "content": ... }` entries.
- `composed_deps` *(optional)* — names of other packages this one composes with.
- `org_id` *(optional)* — attach the package to one of your organizations
  instead of owning it personally.

Returns `201` with `{ "name": "billing-pipeline" }` and a `Location` header.
Creating a name that already exists returns `409`.

### Upload a package as a zip

Send the raw `.zip` bytes (not multipart) with `Content-Type: application/zip`:

```sh
curl -X POST "$FKST_API/api/v1/packages/billing-pipeline/archive" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/zip" \
  --data-binary @billing-pipeline.zip
```

A root `composed.deps` file in the zip becomes the package's composed
dependencies; a root `fkst.env` file is not allowed.

### List, fetch, update, delete

```sh
# Names you can see
curl -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/packages"

# One package with its files
curl -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/packages/billing-pipeline"

# Replace its files
curl -X PUT "$FKST_API/api/v1/packages/billing-pipeline" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "files": [ { "path": "departments/billing/main.lua", "content": "return {}" } ], "composed_deps": [] }'

# Delete it (fails with 409 if a session is currently using it)
curl -X DELETE -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/packages/billing-pipeline"
```

> **Updates are snapshot-based:** a `PUT` only affects sessions you start
> *after* the change. Sessions already running keep the files they began with.

### Size limits

| Limit | Value |
|-------|-------|
| Files per package | 256 |
| Single file content | 1 MiB |
| Total content | 12 MiB |
| Composed dependencies | 256 entries, 256 bytes each |

File paths must use forward slashes and stay inside the package (no absolute
paths, no `..`). All content must be valid UTF-8.

### Generate a package with AI

Describe what you want in plain English and let the service draft a package for
you:

```sh
curl -X POST "$FKST_API/api/v1/packages/generate" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "description": "a department that greets every tick event", "save": true }'
```

- `description` — what the package should do (1–8192 characters).
- `name` *(optional)* — a name for it; otherwise a unique `gen-…` name is minted.
- `save` *(optional)* — when `true`, the draft is saved as your own package if it
  passes validation.

The response (always `200` when generation runs) includes the drafted
`package`, a `validation` verdict, an engine `conformance` check, and whether it
was `saved`:

```json
{
  "package": { "name": "gen-1a2b3c4d", "files": [ /* ... */ ], "composed_deps": [] },
  "validation": { "ok": true, "errors": [] },
  "conformance": { "status": "ok", "errors": [], "skipped_reason": null },
  "saved": true,
  "save_error": null,
  "attempts": 1
}
```

A generated package is held to exactly the same rules as one you upload
yourself. Your description and the generated content are never logged. If this
feature isn't enabled on your deployment, the endpoint returns `503`.

### Share a package

Give other users or an organization access to a package you manage.

| Method | Path | Does |
|--------|------|------|
| `POST` | `/api/v1/packages/{name}/shares` | Share with a user or org |
| `GET` | `/api/v1/packages/{name}/shares` | List a package's shares |
| `DELETE` | `/api/v1/packages/{name}/shares/{share_id}` | Revoke a share |

```sh
curl -X POST "$FKST_API/api/v1/packages/billing-pipeline/shares" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "grantee_kind": "user", "grantee_id": "<nyxid-user-id>", "level": "use" }'
```

- `grantee_kind` — `user` or `org`.
- `grantee_id` — the NyxID user or organization id to share with.
- `level` — `read` (can view) or `use` (can view **and** run sessions).

---

## Sessions: run a package

A **session** runs a package on the fkst engine. You start one, watch its
status, and stop it when you're done.

| Method | Path | Does |
|--------|------|------|
| `POST` | `/api/v1/sessions` | Start a session for a package |
| `GET` | `/api/v1/sessions/{id}` | Check a session's status |
| `POST` | `/api/v1/sessions/{id}/stop` | Request a stop |

Starting a session requires at least **use**-level access to the package.

```sh
# Start
curl -X POST "$FKST_API/api/v1/sessions" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "package_name": "billing-pipeline" }'
# 201 -> { "id": "f4e2c0a1-...", "status": "pending" }

# Poll
curl -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/sessions/f4e2c0a1-..."

# Stop
curl -X POST -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/sessions/f4e2c0a1-.../stop"
# 202 -> { "status": "stopping" }
```

A status response looks like this:

```json
{
  "id": "f4e2c0a1-...",
  "package_name": "billing-pipeline",
  "package_names": ["billing-pipeline"],
  "status": "running",
  "error": null,
  "created_at": "2026-06-15T12:00:00Z",
  "started_at": "2026-06-15T12:00:03Z",
  "stopped_at": null
}
```

(The response also carries runtime diagnostics, and goal-triggered sessions add
`goal_id`, `repo`, and `triggered_by`.)

**Status lifecycle:**

```
pending → validating → running → stopping → stopped
                                         ↘ failed
```

`stop` is asynchronous — it returns `202` immediately, then keep polling `GET`
until the status reaches `stopped`. If a session ends in `failed`, the `error`
field explains why. Only one live session can hold a given package at a time; a
second start returns `409` until the first one stops.

---

## Goals: run against a GitHub repository

A **goal** captures an intent (a prompt), the package(s) to run it with, and an
optional target GitHub repo. You can edit it over time and **trigger** it to
spawn a session whenever you want.

| Method | Path | Does |
|--------|------|------|
| `POST` | `/api/v1/goals` | Create a goal |
| `GET` | `/api/v1/goals` | List your goals (`?status=`, `?limit=`, `?offset=`) |
| `GET` | `/api/v1/goals/{id}` | Fetch one goal |
| `PATCH` | `/api/v1/goals/{id}` | Update a goal |
| `DELETE` | `/api/v1/goals/{id}` | Delete a goal |
| `POST` | `/api/v1/goals/{id}/trigger` | Trigger a goal (starts a session) |

```sh
curl -X POST "$FKST_API/api/v1/goals" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{
    "title": "Build a billing pipeline",
    "description": "Create a billing pipeline that processes invoices.",
    "package_names": ["billing-pipeline"],
    "repo": { "owner": "acme", "name": "billing" }
  }'
```

- `title` — up to 200 characters.
- `description` — the prompt for the engine (up to 16 KiB).
- `package_names` — 1 to 16 packages you can use.
- `repo` *(optional)* — a GitHub `owner`/`name` to work against.
- `org_id` *(optional)* — attach the goal to an organization.

A goal moves through `not_started → triggered → running → stopped`/`failed`.
You can edit the title and description any time; the packages and repo can only
change while the goal is `not_started`, `stopped`, or `failed`.

### Triggering

```sh
# Use the goal's stored repo (or override it for this run)
curl -X POST "$FKST_API/api/v1/goals/<goal-id>/trigger" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "repo": { "owner": "acme", "name": "billing" } }'
```

To have a brand-new repository created for the goal before it runs, use
`create_new` mode:

```sh
curl -X POST "$FKST_API/api/v1/goals/<goal-id>/trigger" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{
    "repo_mode": "create_new",
    "create": { "name": "new-billing-repo", "private": true, "org_login": "acme" }
  }'
```

Trigger returns `202` with the new `session_id`; poll the session as usual.

---

## GitHub issues hub

Read and manage GitHub issues across **all** of your linked GitHub accounts,
without fkst-hosted ever holding your GitHub token.

| Method | Path | Does |
|--------|------|------|
| `GET` | `/api/v1/github/accounts` | List your linked GitHub accounts |
| `GET` | `/api/v1/github/issues` | Aggregate issues across your accounts |
| `POST` | `/api/v1/github/repos/{owner}/{repo}/issues` | Create an issue |
| `GET` | `/api/v1/github/repos/{owner}/{repo}/issues/{number}` | Fetch one issue |
| `PATCH` | `/api/v1/github/repos/{owner}/{repo}/issues/{number}` | Update an issue |
| `GET` | `/api/v1/github/repos/{owner}/{repo}/issues/{number}/comments` | List comments |
| `POST` | `/api/v1/github/repos/{owner}/{repo}/issues/{number}/comments` | Add a comment |

### See issues across your accounts

```sh
curl -H "Authorization: Bearer $TOKEN" \
  "$FKST_API/api/v1/github/issues?filter=assigned&state=open"
```

Optional query parameters: `accounts` (comma-separated logins to limit to),
`filter` (default `assigned`), `state` (default `open`), `labels`
(comma-separated), `page` (default `1`), `per_page` (default `30`, max `50`).

The response groups issues per account and **always returns `200`** once your
accounts resolve — a slow or rate-limited account is reported in its own
`error` block instead of failing the whole request:

```json
{ "results": [
  { "account": "octocat", "issues": [ /* ... */ ], "page": 1, "per_page": 30, "has_more": true },
  { "account": "hubber",  "issues": [], "page": 1, "per_page": 30, "has_more": false,
    "error": { "kind": "rate_limited", "message": "github rate limited; retry later", "retry_after_secs": 41 } }
] }
```

### Create an issue / add a comment

```sh
curl -X POST "$FKST_API/api/v1/github/repos/acme/billing/issues" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "title": "Invoices double-counted", "body": "Steps to reproduce…", "labels": ["bug"] }'
```

If you have **more than one** GitHub account linked, add an `account` field (the
GitHub login to act as) to single-issue operations; with exactly one account
linked it is chosen automatically.

---

## For maintainers

Running or deploying fkst-hosted yourself is covered separately:

- **Local development** (backend, database, configuration, tests):
  [`backend/README.md`](backend/README.md)
- **Kubernetes deployment**: [`backend/deploy/k8s/README.md`](backend/deploy/k8s/README.md)
- **End-to-end smoke test**: [`scripts/e2e/run-e2e.sh`](scripts/e2e/run-e2e.sh)
  (self-documented in its header)
