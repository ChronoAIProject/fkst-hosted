---
name: fkst-control-plane-operation-manual
description: >-
  Operational manual for an AI agent driving the fkst CONTROL PLANE
  (fkst-control-plane) over its HTTP API, exclusively through the NyxID CLI
  credential proxy (`nyxid proxy request …`). Covers the five core user flows —
  create a GitHub repo under my account, bootstrap a repo's `.fkst/` directory,
  trigger a fkst-substrate session with my input arguments, check the status of
  all my substrate sessions, and stop/terminate a specific session — with the
  EXACT request body and response shape for every call so the agent never has to
  guess. Invoke when the user says "create a repo via the control plane",
  "bootstrap .fkst", "trigger a substrate session", "start/stop a fkst session",
  "check my sessions", or "drive the fkst control plane through nyxid".
version: "0.1"
homepage: https://github.com/ChronoAIProject/fkst-hosted
user-invocable: /fkst-control-plane-operation-manual
metadata:
  category: plain
  tag:
    - fkst
    - control-plane
    - fkst-substrate
    - sessions
    - goals
    - repos
    - nyxid
    - http-api
---

# fkst control-plane operation manual

> **You are an AI agent reading this manual to operate the fkst control plane.**
> Throughout, *"you"* / *"my"* means the calling agent acting for the user. The
> **control plane** (`fkst-control-plane`) is ChronoAI's hosted orchestrator: it
> owns **goals** (an intent + packages + a target GitHub repo, mirrored as a
> GitHub issue) and the **sessions** that run the **fkst-substrate** engine
> against those repos. Everything is a JSON HTTP API under `/api/v1`.

This manual documents **five flows only**, each end-to-end with the exact input
and output:

1. Create a new GitHub repo under my account.
2. Bootstrap the `.fkst/` directory into a repo.
3. Trigger a fkst-substrate session with my input arguments.
4. Check the status of all my substrate sessions.
5. Stop and terminate a specific substrate session.

Every fact below is taken from the control-plane source, not from prose docs.

---

## 0. Transport, identity, and conventions (read first)

### 0.1 The control plane is reachable ONLY through the NyxID proxy

`fkst-control-plane` is deployed as a NyxID **downstream service** with **no
public ingress** (its Kubernetes `Service` is `ClusterIP`). You cannot curl it
directly. Every call in this manual goes through the **NyxID CLI**:

```bash
nyxid proxy request <slug> <path> -m <METHOD> [-H "Content-Type: application/json"] [-d '<json-body>']
```

- `<slug>` — the slug the control plane is registered under in NyxID. This is an
  operator-set registration value, **not** hard-coded in the service. Discover it
  with `nyxid service list` (look for the fkst control-plane entry); this manual
  uses the shell variable **`$CP`** for it:

  ```bash
  export CP="fkst-control-plane"   # replace with your deployment's actual slug
  ```
- `<path>` — the API path, **including the `/api/v1` prefix** (paths are relative
  to the registered `endpoint_url`, which is the deployment base before
  `/api/v1`).
- JSON bodies **must** be sent as `application/json`. Set the header explicitly;
  a non-JSON body where JSON is expected is a `400`.

If the NyxID CLI is not yet logged in / the service is not registered, load the
**`nyxid`** skill first (login, `service add`, approvals). Never paste raw tokens
into chat.

### 0.2 How the proxy authenticates you (and why a call may be denied)

The control plane does **not** verify a user token itself. The NyxID proxy
authenticates you and injects your identity as request headers, which the control
plane decodes:

- **`X-NyxID-Identity-Token`** — a JWT whose payload carries your `sub`, `email`,
  `name`, `roles[]`, **`permissions[]`**, and `groups[]`. The control plane
  **decodes but does not re-verify** it (the proxy already verified it).
- Authorization is driven entirely by **`permissions[]`** — exact `fkst:*`
  strings (below). If your identity carries no `fkst:*` permission ("headers
  mode"), **every gated action returns `403`**. NyxID owns which permissions you
  get; if you hit an unexpected `403`, your NyxID role is missing that `fkst:*`
  permission.

| Permission string | Gates |
|-------------------|-------|
| `fkst:admin` | Bypasses every check below. |
| `fkst:goal:read` | List/read goals. |
| `fkst:goal:create` | Create a goal. |
| `fkst:goal:trigger` | Trigger a goal → spawn a session (and create-new-repo). |
| `fkst:repo:setup` | Scaffold `.fkst/` into a repo. |
| `fkst:session:read` | Read a session. |
| `fkst:session:stop` | Stop a session. |

On top of the permission ("action") check, every object is also checked for
**ownership**: you can act on goals/sessions you own; org members with a
writer role can act on org-owned ones; otherwise `403`.

### 0.3 The forwarded bearer (needed for repo creation)

Some actions need your **raw access token forwarded** to the control plane (it
re-uses it to call GitHub via NyxID on your behalf). The proxy **strips the
bearer by default**. If it is absent when required, you get:

```
401 {"error":"unauthorized","message":"this action requires a forwarded user access token"}
```

This affects **Flow 1 (create repo)**. If you hit that `401`, the NyxID proxy
registration for `$CP` must be configured to **forward the Authorization
bearer** (token forwarding / delegation enabled).

### 0.4 The universal error envelope

Every error is one JSON object:

```json
{"error":"<machine_code>","message":"<human text>"}
```

| HTTP | `error` code | Typical cause |
|------|--------------|---------------|
| 400 | `invalid_request` | Bad/unknown JSON field, malformed UUID, bad name/field |
| 401 | `unauthorized` | Missing proxy identity, or missing forwarded bearer (sets `WWW-Authenticate: Bearer`) |
| 403 | `forbidden` | Missing `fkst:*` permission, or not the owner |
| 404 | `not_found` | No such goal/session/repo (or hidden from you) |
| 409 | `conflict` | Goal already triggered/running; repo name taken |
| 422 | `unprocessable` | GitHub App not installed; no repo on goal; pre-flight failure |
| 429 | `rate_limited` | GitHub upstream rate-limited (sets `Retry-After`) |
| 500 | `internal` | Server error — message is always the fixed `"internal server error"` |
| 502 | `upstream_error` | GitHub returned an unexpected error |
| 503 | `unavailable` | NyxID / credential proxy / dependency down |

> **Conventions.** Unknown JSON fields are rejected (`deny_unknown_fields`).
> Timestamps are RFC 3339 UTC (`…Z`). Goal and session IDs are UUIDs; a malformed
> UUID in a path is `400`, never `404`. Never log or echo a secret value or the
> goal `description` (the engine prompt).

### 0.5 The mental model (so the flows make sense)

```
goal  ──trigger──▶  session  ──runs──▶  fkst-substrate engine against a repo
 │                    │
 │ (a goal is a       │ (one active session per goal at a time;
 │  GitHub issue)     │  goal.active_session_id points to it)
```

- A **session is never created directly.** It is always spawned by **triggering a
  goal**. So most flows start by ensuring a goal exists.
- There is **no "list sessions" endpoint** and **no "create repo only"
  endpoint** — Flows 1 and 4 are built from the primitives below. This is called
  out where it matters so you are not surprised.

---

## Flow 1 — Create a new GitHub repo under my account

### What actually happens (read this first)

There is **no standalone "create repo" endpoint.** A repo is created as a side
effect of **triggering a goal in `create_new` mode**: the control plane proxies
GitHub's "create repository" call through NyxID (`POST /user/repos`, or
`/orgs/{org}/repos` with `org_login`, always with `auto_init: true`) **and then
immediately spawns a substrate session against the new repo.** If you want a repo
*and* a first run, this is one call. There is no way to create the repo without
also starting a session.

### Prerequisites

- An existing **goal** to trigger (create one first — see Flow 3 §3a step 1).
- Permission **`fkst:goal:trigger`** (+ you own the goal, or are an org writer).
- Your **bearer must be forwarded** (see §0.3) — repo creation acts as you
  against GitHub. Without it → `401`.
- The NyxID credential proxy must be configured (else `503`).
- The fkst-hosted **GitHub App** must end up installed on the new repo (GitHub
  requires interactive consent; the call returns an actionable `422` if not).

### Request

`POST /api/v1/goals/{goal_id}/trigger` with `repo_mode: "create_new"`.

```bash
nyxid proxy request "$CP" "/api/v1/goals/$GOAL_ID/trigger" -m POST \
  -H "Content-Type: application/json" \
  -d '{
    "repo_mode": "create_new",
    "create": {
      "name": "my-new-repo",
      "private": true,
      "description": "optional description",
      "org_login": null
    }
  }'
```

**`create` object** (required for `create_new`; the top-level `repo` field is
**forbidden** in this mode):

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `name` | string | yes | Repo name; `^[A-Za-z0-9._-]{1,100}$`. |
| `private` | bool | no | **Defaults to `true`** when omitted. |
| `description` | string\|null | no | Optional repo description. |
| `org_login` | string\|null | no | Create under this org; omit/`null` → under **your** personal account. |

You may also pass `secrets` and `ornn_skills` here (they apply to the session
that is spawned — see Flow 3 §3.2).

### Success response — `202 Accepted`

```json
{
  "goal_id": "f4e2c0a1-9b3d-4d2e-8c11-3a6b5e0d7f12",
  "session_id": "8c11f4e2-3a6b-4d2e-9b3d-5e0d7f12c0a1",
  "goal_status": "triggered",
  "session_status": "pending"
}
```

The repo now exists; the session is queued. Poll its status with **Flow 4 /
Flow 5's `GET /api/v1/sessions/{session_id}`**.

### Notable error responses

| HTTP | Meaning |
|------|---------|
| `400 invalid_request` | `create_new` with a `repo` field present, or `create` missing, or a bad repo name. |
| `401 unauthorized` | Bearer not forwarded (`"this action requires a forwarded user access token"`), or NyxID token exchange rejected. |
| `409 conflict` | Repo **name already exists** on the account. |
| `422 unprocessable` | Repo was created but the **GitHub App is not installed / pending org-owner approval** (message includes the install hint); or your GitHub connection is **missing the `repo` scope**; or org **SAML SSO**/policy blocks creation. |
| `503 unavailable` | Credential proxy unavailable, or GitHub rate-limited. |

> Idempotency: if the goal already references a repo matching `create.name` (and
> `org_login`), creation is skipped and that repo is reused.

---

## Flow 2 — Bootstrap the `.fkst/` directory into a repo

### What it does

Initializes an **existing** GitHub repo for fkst by committing a `.fkst/`
directory onto the repo's **default branch**, using the fkst-hosted GitHub App's
installation token. The scaffold is three files: a conformant `example` package
(its `departments/example/main.lua` entry + a `README.md`) plus the per-repo
`AGENTS.md`. Idempotent and non-destructive by default.

### Prerequisites

- The repo already exists and the fkst-hosted **GitHub App is installed** on it
  (else `422` with an install hint).
- Permission **`fkst:repo:setup`** (this is its own grant — having
  `fkst:goal:create` does **not** grant it).

### Request

`POST /api/v1/repos/{owner}/{name}/fkst-setup` (optional `?force=true`).

```bash
nyxid proxy request "$CP" "/api/v1/repos/$OWNER/$NAME/fkst-setup" -m POST \
  -H "Content-Type: application/json" \
  -d '{}'
```

- Query param **`force`** (`?force=true`): re-commit the three scaffold paths
  over an existing `.fkst` (never deletes other `.fkst` content). Default
  (absent/`false`) is the safe no-overwrite path.
- Optional body field **`org_id`** (string): if set, you must be a writer of that
  org. Send `{}` (or omit the body) when not scoping to an org.

### Success responses

**Fresh repo — `201 Created`:**

```json
{
  "repo": { "owner": "my-account", "name": "my-new-repo" },
  "default_branch": "main",
  "commit_sha": "a1b2c3d4e5f6...",
  "created_paths": [
    ".fkst/packages/example/departments/example/main.lua",
    ".fkst/packages/example/README.md",
    ".fkst/AGENTS.md"
  ],
  "already_initialized": false
}
```

**Already initialized, not forcing — `200 OK`** (no write performed):

```json
{
  "repo": { "owner": "my-account", "name": "my-new-repo" },
  "default_branch": null,
  "commit_sha": null,
  "created_paths": [],
  "already_initialized": true
}
```

(A `?force=true` re-commit over existing `.fkst` also returns `200`, but with the
populated `default_branch`/`commit_sha`/`created_paths` and
`already_initialized: false`.)

### Notable error responses

| HTTP | Meaning |
|------|---------|
| `400 invalid_request` | Malformed `owner`/`name` (owner `^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})$`, name `^[A-Za-z0-9._-]{1,100}$`). |
| `403 forbidden` | Missing `fkst:repo:setup`, or not an org writer (when `org_id` given). |
| `404 not_found` | Repo not found on the write path. |
| `409 conflict` | The default branch moved mid-scaffold — retry. |
| `422 unprocessable` | GitHub App not installed on the repo, or App not configured on the deployment, or repo not visible to the App. |
| `502 upstream_error` | GitHub rejected the scaffold commit. |

---

## Flow 3 — Trigger a fkst-substrate session with my input arguments

A session always comes from triggering a goal. Two shapes:

- **§3a — Trigger an existing goal** (`POST /goals/{id}/trigger`). Use when you
  already created a goal and want to (re)run it.
- **§3b — One-shot submit** (`POST /goals/submit`). Creates the goal + its issue
  **and** triggers, in a single call (issue-driven or fully inline).

### 3.1 Your "input arguments"

Both shapes accept the same two argument bundles:

**`secrets[]`** — env vars injected into the engine for this session. Held in the
control plane **in memory only** (never persisted), scoped to the target repo.

```json
{ "key": "OPENAI_API_KEY", "value": "sk-…", "kind": "secret" }
```
| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `key` | string | yes | Env-var name (rejected if reserved/invalid/oversized → `422`). |
| `value` | string | yes | The value (redacted in all logs). |
| `kind` | `"secret"`\|`"variable"` | no | **Defaults to `"secret"`.** Use `"variable"` for non-sensitive values. |

**`ornn_skills[]`** — Ornn skills/skillsets to inject into the session's codex.

```json
{ "kind": "skill", "name": "my-skill", "version": "1.4" }
```
| Field | Type | Notes |
|-------|------|-------|
| `kind` | `"skill"`\|`"skillset"` | Lowercase. |
| `name` | string | Artifact name. |
| `version` | string | **Concrete** `"<major>.<minor>"` — no `latest`/dist-tags. |

### 3a — Trigger an existing goal

**Step 1 — Create the goal** (if you don't have one). `POST /api/v1/goals`,
permission **`fkst:goal:create`**:

```bash
nyxid proxy request "$CP" "/api/v1/goals" -m POST \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Build the billing pipeline",
    "description": "Implement a billing pipeline that processes invoices.",
    "package_names": ["billing-pipeline"],
    "repo": { "owner": "my-account", "name": "my-new-repo" },
    "org_id": null
  }'
```
Limits: `title` 1–200 chars; `description` 1–16384 bytes; `package_names` 1–16,
each `^[A-Za-z0-9_-]+$` and ≤128 bytes. `repo` and `org_id` optional.

**`201 Created`** returns a **GoalView** (note `id` and `active_session_id`):

```json
{
  "id": "f4e2c0a1-9b3d-4d2e-8c11-3a6b5e0d7f12",
  "title": "Build the billing pipeline",
  "description": "Implement a billing pipeline that processes invoices.",
  "package_names": ["billing-pipeline"],
  "repo": { "owner": "my-account", "name": "my-new-repo" },
  "status": "not_started",
  "owner_user_id": "user-123",
  "org_id": null,
  "active_session_id": null,
  "created_at": "2026-06-17T10:00:00Z",
  "updated_at": "2026-06-17T10:00:00Z"
}
```

**Step 2 — Trigger it.** `POST /api/v1/goals/{id}/trigger`, permission
**`fkst:goal:trigger`**:

```bash
nyxid proxy request "$CP" "/api/v1/goals/$GOAL_ID/trigger" -m POST \
  -H "Content-Type: application/json" \
  -d '{
    "repo_mode": "existing",
    "repo": { "owner": "my-account", "name": "my-new-repo" },
    "secrets": [
      { "key": "OPENAI_API_KEY", "value": "sk-…", "kind": "secret" },
      { "key": "LOG_LEVEL", "value": "debug", "kind": "variable" }
    ],
    "ornn_skills": [
      { "kind": "skill", "name": "my-skill", "version": "1.4" }
    ]
  }'
```

Trigger body (all fields optional; `deny_unknown_fields`):

| Field | Type | Notes |
|-------|------|-------|
| `repo_mode` | `"existing"`\|`"create_new"` | **Defaults to `"existing"`.** (`create_new` → Flow 1.) |
| `repo` | `{owner,name}` | **Only in `existing` mode**; overrides the goal's stored repo. Omit to use the goal's repo. |
| `create` | object | **Only in `create_new` mode** (Flow 1); forbidden here. |
| `secrets` | array | §3.1. |
| `ornn_skills` | array | §3.1. |

**`202 Accepted` — TriggerResponse:**

```json
{
  "goal_id": "f4e2c0a1-9b3d-4d2e-8c11-3a6b5e0d7f12",
  "session_id": "8c11f4e2-3a6b-4d2e-9b3d-5e0d7f12c0a1",
  "goal_status": "triggered",
  "session_status": "pending"
}
```

### 3b — One-shot submit (create + trigger together)

`POST /api/v1/goals/submit` requires **both** `fkst:goal:create` **and**
`fkst:goal:trigger`. The body is tagged on `source`.

**Inline source** — supply everything; the server files an issue and triggers:

```bash
nyxid proxy request "$CP" "/api/v1/goals/submit" -m POST \
  -H "Content-Type: application/json" \
  -d '{
    "source": "inline",
    "goal": "Implement a billing pipeline that processes invoices.",
    "repo": { "owner": "my-account", "name": "my-new-repo" },
    "package_names": ["billing-pipeline"],
    "secrets": [ { "key": "OPENAI_API_KEY", "value": "sk-…" } ],
    "ornn_skills": [ { "kind": "skill", "name": "my-skill", "version": "1.4" } ]
  }'
```
- `goal` — the engine prompt (never logged). `repo` — either `{owner,name}` or
  `{"url":"https://github.com/owner/name"}` (or a bare `owner/name`).
  `package_names` 1–16. `secrets`/`ornn_skills` as §3.1.

**Issue source** — adopt an existing user-authored issue (its `### Goal` body
becomes the prompt); secrets still come inline, never from the issue:

```bash
nyxid proxy request "$CP" "/api/v1/goals/submit" -m POST \
  -H "Content-Type: application/json" \
  -d '{
    "source": "issue",
    "issue": { "url": "https://github.com/my-account/my-new-repo/issues/42" },
    "secrets": []
  }'
```
- `issue` — either `{"url":"…/issues/{n}"}` or `{"owner","name","number"}`.

**`202 Accepted` — SubmitSessionResponse** (TriggerResponse + the issue locator):

```json
{
  "goal_id": "f4e2c0a1-9b3d-4d2e-8c11-3a6b5e0d7f12",
  "session_id": "8c11f4e2-3a6b-4d2e-9b3d-5e0d7f12c0a1",
  "issue_number": 42,
  "issue_url": "https://github.com/my-account/my-new-repo/issues/42",
  "goal_status": "triggered",
  "session_status": "pending"
}
```

> Submit always targets an **existing** repo — it does not create one. For a new
> repo use Flow 1.

### Notable error responses (both shapes)

| HTTP | Meaning |
|------|---------|
| `400 invalid_request` | Bad field (inline path), unknown field, or malformed UUID. |
| `403 forbidden` | Missing the required permission, or you are not the goal owner / org writer. |
| `409 conflict` | Goal is already `triggered`/`running` (a live session holds it). |
| `422 unprocessable` | No effective repo on the goal; or **pre-flight failure** — one aggregated `422` listing every problem (missing `.fkst/packages/<name>/`, unavailable Ornn pin, GitHub App not installed on the repo); or a bad issue/repo reference (submit). |
| `503 unavailable` | Dependency down, or the goal issue could not be filed (inline submit). |

After a `202`, **poll the session** (Flow 4) until it leaves `pending`.

---

## Flow 4 — Check the status of all my substrate sessions

### What actually happens (read this first)

There is **no "list all sessions" endpoint.** A session is owned by a goal, so
"all my sessions" = **list my goals, then read the active session of each.**

### Step 1 — List my goals

`GET /api/v1/goals`, permission **`fkst:goal:read`**. Returns goals you own plus
goals of orgs you can see.

```bash
nyxid proxy request "$CP" "/api/v1/goals" -m GET
# optional filters:
nyxid proxy request "$CP" "/api/v1/goals?status=running&limit=50&offset=0" -m GET
```

Query params: `status` (a valid goal status; invalid → `400`), `limit` (default
50, max 200), `offset` (default 0).

**`200 OK`** — an array of **GoalView** (same shape as Flow 3). The field that
links to the live session is **`active_session_id`**:

```json
[
  {
    "id": "f4e2c0a1-9b3d-4d2e-8c11-3a6b5e0d7f12",
    "title": "Build the billing pipeline",
    "description": "…",
    "package_names": ["billing-pipeline"],
    "repo": { "owner": "my-account", "name": "my-new-repo" },
    "status": "running",
    "owner_user_id": "user-123",
    "org_id": null,
    "active_session_id": "8c11f4e2-3a6b-4d2e-9b3d-5e0d7f12c0a1",
    "created_at": "2026-06-17T10:00:00Z",
    "updated_at": "2026-06-17T10:05:00Z"
  }
]
```

Collect every non-null `active_session_id`. (Goals with `active_session_id:
null` have no live session right now.)

### Step 2 — Read each session

`GET /api/v1/sessions/{session_id}`, permission **`fkst:session:read`**:

```bash
nyxid proxy request "$CP" "/api/v1/sessions/$SESSION_ID" -m GET
```

**`200 OK`** — **SessionView**:

```json
{
  "id": "8c11f4e2-3a6b-4d2e-9b3d-5e0d7f12c0a1",
  "package_name": "billing-pipeline",
  "status": "running",
  "pod_id": "fkst-worker-7d9c",
  "fencing_token": 42,
  "pid": 1234,
  "runtime_dir": "/var/run/fkst/8c11f4e2",
  "error": null,
  "owner_user_id": "user-123",
  "org_id": null,
  "goal_id": "f4e2c0a1-9b3d-4d2e-8c11-3a6b5e0d7f12",
  "repo": { "owner": "my-account", "name": "my-new-repo" },
  "triggered_by": "user-123",
  "package_names": ["billing-pipeline"],
  "created_at": "2026-06-17T10:05:00Z",
  "started_at": "2026-06-17T10:05:03Z",
  "stopped_at": null
}
```

- **`status`** is one of: `pending` → `validating` → `running` → `stopping` →
  `stopped`, or `failed` on any error (read `error` for the reason).
- **`terminal_cause`** appears **only once the session is terminal** (omitted
  while live): `terminated` (you stopped it), `completed` (engine finished
  cleanly), or `failed`.
- Unset fields serialize as explicit `null`; timestamps end in `Z`.

### Notable error responses

| HTTP | Meaning |
|------|---------|
| `400 invalid_request` | Malformed session UUID, or invalid `status` filter on the goals list. |
| `403 forbidden` | Missing `fkst:goal:read` / `fkst:session:read`, or the session isn't yours. |
| `404 not_found` | No such session (or hidden from you). |

---

## Flow 5 — Stop and terminate a specific substrate session

### Request

`POST /api/v1/sessions/{session_id}/stop`, permission **`fkst:session:stop`**
(+ you own the session, or are an org writer). No body required.

```bash
nyxid proxy request "$CP" "/api/v1/sessions/$SESSION_ID/stop" -m POST
```

### Success response — `202 Accepted`

```json
{ "status": "stopping" }
```

The `202` only **acknowledges** the request — the stop is asynchronous and
**idempotent** (calling it again, or stopping an already-stopping/stopped
session, still returns `202`). It does not block until the engine exits.

### Confirm termination

Poll `GET /api/v1/sessions/{session_id}` (Flow 4 step 2) until:

```json
{ "status": "stopped", "terminal_cause": "terminated", "stopped_at": "2026-06-17T10:30:00Z", "...": "..." }
```

`status: "stopped"` with `terminal_cause: "terminated"` confirms a user-initiated
stop completed. (`completed` would mean the engine finished on its own;
`failed` means it errored.)

### Notable error responses

| HTTP | Meaning |
|------|---------|
| `400 invalid_request` | Malformed session UUID. |
| `403 forbidden` | Missing `fkst:session:stop`, or the session isn't yours. |
| `404 not_found` | No such session (or hidden from you). |

---

## Appendix — enumerations & quick map

**Goal status:** `not_started` · `triggered` · `running` · `stopped` · `failed`
**Session status:** `pending` · `validating` · `running` · `stopping` · `stopped` · `failed`
**Session terminal_cause** (terminal only): `terminated` · `completed` · `failed`
**Trigger `repo_mode`:** `existing` (default) · `create_new`
**Secret `kind`:** `secret` (default) · `variable`
**Ornn pin `kind`:** `skill` · `skillset`

| Flow | Method & path | Permission |
|------|---------------|------------|
| 1 — create repo | `POST /api/v1/goals/{id}/trigger` (`repo_mode:create_new`) | `fkst:goal:trigger` + forwarded bearer |
| 2 — bootstrap `.fkst/` | `POST /api/v1/repos/{owner}/{name}/fkst-setup` | `fkst:repo:setup` |
| 3a — create goal | `POST /api/v1/goals` | `fkst:goal:create` |
| 3a — trigger goal | `POST /api/v1/goals/{id}/trigger` (`repo_mode:existing`) | `fkst:goal:trigger` |
| 3b — submit (create+trigger) | `POST /api/v1/goals/submit` | `fkst:goal:create` + `fkst:goal:trigger` |
| 4 — list goals | `GET /api/v1/goals` | `fkst:goal:read` |
| 4 — read session | `GET /api/v1/sessions/{id}` | `fkst:session:read` |
| 5 — stop session | `POST /api/v1/sessions/{id}/stop` | `fkst:session:stop` |

> All paths run through `nyxid proxy request "$CP" <path> -m <METHOD>`. `$CP` is
> the control plane's NyxID slug. After any `202`, poll `GET
> /api/v1/sessions/{id}` for the real state.
