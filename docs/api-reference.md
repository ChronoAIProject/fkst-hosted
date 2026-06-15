# fkst-hosted HTTP API Reference

This is the complete reference for the **fkst-hosted** HTTP API — every public
endpoint, with its path, required headers, permissions, request/response
shapes, and examples. For a high-level overview of the project, see the
[root README](../README.md); to run or deploy the service, see
[`backend/README.md`](../backend/README.md).

- [Conventions](#conventions)
- [Authentication & authorization](#authentication--authorization)
- [Errors](#errors)
- [Health](#health)
- [Packages](#packages)
- [Package generation](#package-generation)
- [Package sharing](#package-sharing)
- [Sessions](#sessions)
- [Goals](#goals)
- [GitHub issues hub](#github-issues-hub)
- [Vault (env variables & secrets)](#vault-env-variables--secrets)
- [Appendix: data types & limits](#appendix-data-types--limits)

---

## Conventions

| Topic | Detail |
|-------|--------|
| **Base URL** | All application endpoints live under `/api/v1`. Examples use `$FKST_API` for the deployment base (e.g. `https://fkst.example.com`). |
| **Content type** | JSON request bodies require `Content-Type: application/json`. Zip uploads require `Content-Type: application/zip`. Responses are JSON unless noted. |
| **Unknown fields** | Request bodies reject unknown JSON fields with `400` — a typo such as `"file"` for `"files"` fails loudly rather than being silently ignored. |
| **Timestamps** | All timestamps are RFC 3339 / ISO 8601 UTC strings ending in `Z` (e.g. `2026-06-15T12:00:00Z`). |
| **IDs** | Session and goal IDs are UUIDs. A malformed UUID in a path is a `400`, never a `404`. |
| **Request ID** | Every response carries an `x-request-id` header (echoed from the request if supplied, otherwise generated) for correlation in logs. |
| **CORS** | Cross-origin requests are accepted (`GET, POST, PUT, PATCH, DELETE`). |
| **Request timeout** | Requests exceeding the server's configured budget return `408 Request Timeout`. |

---

## Authentication & authorization

### Authentication

Every endpoint **except** the [health checks](#health) requires a **NyxID access
token** (an RS256 JWT) sent as a bearer token:

```
Authorization: Bearer <nyxid-access-token>
```

- A missing or malformed `Authorization` header, or an invalid/expired token,
  returns `401 Unauthorized` with a `WWW-Authenticate: Bearer` header.
- If the authentication service (NyxID JWKS) is unreachable, requests return
  `503 unavailable`.

The token's subject (`sub`) identifies the caller (`user_id`); its `scope` claim
carries OAuth2 scopes. The scope `fkst:admin` is an operator escape hatch that
bypasses all resource checks.

### Authorization model

Resources (packages, sessions, goals) carry an optional `owner_user_id` and an
optional `org_id`. Access to an action is decided in this order:

1. **Admin scope** — a caller with the `fkst:admin` scope may do anything.
2. **Owner** — the `owner_user_id` may do anything to their resource.
3. **Organization role** — when the resource has an `org_id`, the caller's role
   in that org (resolved via NyxID) grants:

   | Org role | Read | Write | Manage |
   |----------|:----:|:-----:|:------:|
   | Viewer | ✅ | — | — |
   | Member | ✅ | ✅ | — |
   | Admin | ✅ | ✅ | ✅ |

4. **Legacy** — resources with no `owner_user_id` (created before auth existed)
   are open to any authenticated caller.

The three actions map to endpoints as:

- **Read** — fetching a resource.
- **Write** — updating a resource.
- **Manage** — deleting a resource or managing its shares.

**Packages** additionally support **shares** (see [Package sharing](#package-sharing)):
a `read`-level share grants Read; a `use`-level share grants the ability to
**run a session** with the package. A `read` share does **not** grant `use`.

**Anti-enumeration:** a denied **Read** on someone else's private resource
returns `404 not_found` (identical to a resource that doesn't exist), so the API
never reveals that a resource you can't see exists. Denied **Write**/**Manage**
returns `403 forbidden`.

---

## Errors

All errors share one JSON envelope:

```json
{ "error": "not_found", "message": "package not found: billing-pipeline" }
```

`error` is a stable machine code; `message` is human-readable. Internal failures
return a fixed `"internal server error"` message (details are logged, never sent
to the client).

| HTTP status | `error` code | Meaning | Special headers |
|-------------|--------------|---------|-----------------|
| `400` | `invalid_request` | Malformed body, invalid name/field, or unknown JSON field | |
| `401` | `unauthorized` | Missing/invalid token | `WWW-Authenticate: Bearer` |
| `403` | `forbidden` | Authenticated but not permitted | |
| `404` | `not_found` | Resource absent, or hidden by anti-enumeration | |
| `409` | `conflict` | Conflicts with current state (duplicate name, busy package, illegal status transition) | |
| `422` | `unprocessable` | Understood but cannot proceed (e.g. GitHub App not installed; dependent resource missing) | |
| `429` | `rate_limited` | Upstream (GitHub) rate-limited the request | `Retry-After: <seconds>` |
| `500` | `internal` | Unexpected server error | |
| `502` | `upstream_error` | An upstream provider (GitHub via proxy) returned an unexpected error | |
| `503` | `unavailable` | A dependency (database, NyxID, LLM gateway, credential proxy) is unavailable | |
| `408` | — | Request exceeded the server timeout | |

---

## Health

Liveness plus a real database ping. **No authentication required.**

### `GET /health` · `GET /api/v1/health`

- **Auth:** none.
- **Headers:** none required.

**Responses**

| Status | Body |
|--------|------|
| `200 OK` | `{ "status": "ok", "mongo": "up", "version": "<build version>" }` |
| `503 Service Unavailable` | `{ "status": "degraded", "mongo": "down", "version": "<build version>" }` |

```sh
curl "$FKST_API/health"
# {"status":"ok","mongo":"up","version":"0.0.0"}
```

---

## Packages

A **package** is the unit the fkst engine runs: a `name` plus a list of lua
`files` and an optional list of `composed_deps`. A package must contain at least
one **engine entry file**:

- a department entry — `departments/<name>/main.lua`, or
- a raiser entry — `raisers/<name>.lua`.

**Common data shapes**

```jsonc
// PackageFile
{ "path": "departments/billing/main.lua", "content": "return {}" }

// Package (response body for GET / PUT)
{
  "name": "billing-pipeline",
  "files": [ { "path": "...", "content": "..." } ],
  "composed_deps": [],
  "owner_user_id": "user-123",   // null for legacy packages
  "org_id": null,                // null for personal packages
  "created_at": "2026-06-15T12:00:00Z",
  "updated_at": "2026-06-15T12:00:00Z"
}
```

See [size limits](#appendix-data-types--limits) for the exact constraints on
names, files, and dependencies.

---

### `POST /api/v1/packages` — create a package

Create a package from JSON.

- **Permission:** any authenticated caller. If `org_id` is supplied, the caller
  must be an **org Member or Admin** of that org (else `403`).
- **Headers:** `Authorization: Bearer …`, `Content-Type: application/json`.

**Request body**

| Field | Type | Required | Notes |
|-------|------|:--------:|-------|
| `name` | string | yes | Must match `^[A-Za-z0-9_-]+$` |
| `files` | array of `PackageFile` | yes | ≥ 1, must include an engine entry file |
| `composed_deps` | array of string | no | Defaults to `[]` |
| `org_id` | string | no | Attach to an org instead of owning personally |

**Responses**

| Status | Meaning |
|--------|---------|
| `201 Created` | Created. Body `{ "name": "<name>" }`; `Location: /api/v1/packages/<name>` header |
| `400` | Invalid name, files, or validation failure |
| `409` | A package with that name already exists |

```sh
curl -X POST "$FKST_API/api/v1/packages" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{
    "name": "billing-pipeline",
    "files": [ { "path": "departments/billing/main.lua", "content": "return {}" } ],
    "composed_deps": []
  }'
# 201 -> { "name": "billing-pipeline" }
```

---

### `GET /api/v1/packages` — list package names

Returns a flat, ascending JSON array of package names.

- **Permission:** authenticated. Returns packages you can see (owned, your orgs',
  and legacy) plus packages shared with you. An empty store returns `[]`.
- **Headers:** `Authorization: Bearer …`.

**Query parameters**

| Param | Values | Notes |
|-------|--------|-------|
| `filter` | `shared` | When `shared`, returns **only** package names shared with you |

```sh
curl -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/packages"
# ["audit-log","billing-pipeline"]

curl -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/packages?filter=shared"
# ["billing-pipeline"]
```

---

### `GET /api/v1/packages/{name}` — fetch a package

- **Permission:** **Read** — owner, any role in the package's org, a `read`/`use`
  share, admin scope, or a legacy package. A package you can't read returns
  `404` (anti-enumeration).
- **Headers:** `Authorization: Bearer …`.

**Path parameters:** `name` — the package name (`^[A-Za-z0-9_-]+$`).

**Responses:** `200 OK` with a `Package` body; `400` invalid name; `404` not
found / not visible.

```sh
curl -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/packages/billing-pipeline"
```

---

### `PUT /api/v1/packages/{name}` — replace a package's contents

Atomically replaces `files` and `composed_deps`. The name, `created_at`, and
ownership are untouched.

- **Permission:** **Write** — owner, org Member/Admin, or admin scope.
- **Headers:** `Authorization: Bearer …`, `Content-Type: application/json`.

**Request body**

| Field | Type | Required |
|-------|------|:--------:|
| `files` | array of `PackageFile` | yes |
| `composed_deps` | array of string | no (defaults `[]`) |

**Responses:** `200 OK` with the updated `Package`; `400` validation; `403`
not permitted; `404` not found.

> **Snapshot semantics:** sessions materialize a package's files **at start**.
> A `PUT` therefore affects only sessions started **after** it — already-running
> sessions are unaffected.

```sh
curl -X PUT "$FKST_API/api/v1/packages/billing-pipeline" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "files": [ { "path": "departments/billing/main.lua", "content": "return {}" } ], "composed_deps": [] }'
```

---

### `DELETE /api/v1/packages/{name}` — delete a package

- **Permission:** **Manage** — owner, org Admin, or admin scope.
- **Headers:** `Authorization: Bearer …`.

**Responses**

| Status | Meaning |
|--------|---------|
| `204 No Content` | Deleted. All of the package's shares are cascade-removed |
| `403` | Not permitted |
| `404` | Not found |
| `409` | The package has an active session or a live lease — stop it first |

```sh
curl -X DELETE -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/packages/billing-pipeline"
```

---

### `POST /api/v1/packages/{name}/archive` — create from a zip

Create a package by uploading a zip archive as **raw bytes** (not multipart).

- **Permission:** any authenticated caller (the package is owned by you;
  attaching to an org is not supported on this path).
- **Headers:** `Authorization: Bearer …`, `Content-Type: application/zip`.

**Body:** raw `application/zip` bytes.

**Zip rules**

- Up to 256 file entries, plus one optional root `composed.deps`.
- Per-file content ≤ 1 MiB; total decoded content ≤ 12 MiB; all content UTF-8.
- A root `composed.deps` file is parsed into `composed_deps` (not stored as a file).
- A root `fkst.env` file is **rejected** (host-owned).
- Path rules match JSON create (no `..`, `/`-prefixed, backslash, or control chars).

**Responses:** `201 Created` with `{ "name": "<name>" }` and a `Location`
header; `400` invalid zip/validation; `409` name already exists.

```sh
curl -X POST "$FKST_API/api/v1/packages/billing-pipeline/archive" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/zip" \
  --data-binary @billing-pipeline.zip
```

---

### `PUT /api/v1/packages/{name}/archive` — replace from a zip

Same body and zip rules as the archive create.

- **Permission:** **Write** — owner, org Member/Admin, or admin scope.
- **Headers:** `Authorization: Bearer …`, `Content-Type: application/zip`.
- **Responses:** `200 OK` with the updated `Package`; `400`; `403`; `404`.
- Snapshot semantics apply (see `PUT .../packages/{name}`).

---

## Package generation

### `POST /api/v1/packages/generate` — generate a package with AI

Generate a validated fkst package draft from a natural-language description via
NyxID's LLM gateway, and optionally save it as your own package.

- **Permission:** any authenticated caller. With `save: true`, the draft is
  created as **your own** package.
- **Headers:** `Authorization: Bearer …`, `Content-Type: application/json`.
- **Availability:** requires the LLM gateway to be configured on the deployment;
  otherwise the endpoint returns `503`.

**Request body**

| Field | Type | Required | Notes |
|-------|------|:--------:|-------|
| `description` | string | yes | 1–8192 bytes |
| `name` | string | no | Must match `^[A-Za-z0-9_-]+$`; a unique `gen-<hex>` name is minted when absent |
| `save` | boolean | no | When `true`, persist the draft if it validates and conformance did not fail |

**Responses**

`200 OK` whenever generation runs — **even if the draft fails validation or
conformance** (the verdict is in the body):

```json
{
  "package": { "name": "gen-1a2b3c4d", "files": [ /* PackageFile[] */ ], "composed_deps": [] },
  "validation": { "ok": true, "errors": [] },
  "conformance": { "status": "ok", "errors": [], "skipped_reason": null },
  "saved": true,
  "save_error": null,
  "attempts": 1
}
```

- `validation.ok` — passes the same gate every uploaded package passes. On
  failure, one corrective retry (with the errors fed back to the model) is made.
- `conformance.status` — `ok` | `failed` | `skipped` (with `skipped_reason` when
  the engine dry-run could not run, e.g. raiser-only draft or exhausted budget).
- `saved` / `save_error` — when `save: true`, whether it was persisted, or why not.
- `attempts` — `1` or `2`.

| Status | Meaning |
|--------|---------|
| `200` | Generation ran (inspect `validation`/`conformance`) |
| `400` | Empty/oversize description, or an invalid explicit `name` |
| `409` | `save: true` collided with an existing package name |
| `503` | Generation not configured, or the gateway is unreachable |

**Privacy:** the model is reached through NyxID's gateway with a service-account
bearer; fkst-hosted never sees a raw provider key. The LLM has no tool access.
Your description, the prompts, and the raw model output are never logged.

```sh
curl -X POST "$FKST_API/api/v1/packages/generate" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "description": "a department that greets every tick event", "save": true }'
```

---

## Package sharing

Grant other users or organizations access to a package you manage. All share
endpoints require **Manage** permission on the package (owner, org Admin, or
admin scope).

**Data shape**

```jsonc
// ShareView
{
  "id": "8f1c…",                 // share id (UUID)
  "package_name": "billing-pipeline",
  "grantee_kind": "user",        // "user" | "org"
  "grantee_id": "user-456",      // NyxID user id or org id
  "level": "use",                // "read" | "use"
  "granted_by": "user-123",
  "created_at": "2026-06-15T12:00:00Z"
}
```

---

### `POST /api/v1/packages/{name}/shares` — create a share

- **Permission:** Manage on the package.
- **Headers:** `Authorization: Bearer …`, `Content-Type: application/json`.

**Request body**

| Field | Type | Required | Notes |
|-------|------|:--------:|-------|
| `grantee_kind` | `"user"` \| `"org"` | yes | |
| `grantee_id` | string | yes | NyxID user id (must exist) or org id (must exist, and you must be a member) |
| `level` | `"read"` \| `"use"` | yes | `read` = view; `use` = view + run sessions |

**Responses**

| Status | Meaning |
|--------|---------|
| `201 Created` | The created `ShareView` |
| `400` | Sharing with yourself, or an unknown user |
| `403` | Not a member of the target org |
| `409` | A share already exists for that grantee |

```sh
curl -X POST "$FKST_API/api/v1/packages/billing-pipeline/shares" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "grantee_kind": "user", "grantee_id": "user-456", "level": "use" }'
```

---

### `GET /api/v1/packages/{name}/shares` — list shares

- **Permission:** Manage on the package.
- **Responses:** `200 OK` with an array of `ShareView`.

```sh
curl -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/packages/billing-pipeline/shares"
```

---

### `DELETE /api/v1/packages/{name}/shares/{share_id}` — revoke a share

- **Permission:** Manage on the package.
- **Path parameters:** `name`, `share_id` (UUID; the share must belong to the
  named package).
- **Responses:** `204 No Content`; `400` invalid share id; `404` share not found.

```sh
curl -X DELETE -H "Authorization: Bearer $TOKEN" \
  "$FKST_API/api/v1/packages/billing-pipeline/shares/8f1c…"
```

---

## Sessions

A **session** runs a package on the fkst engine. You start one, poll its status,
and stop it. Status lifecycle:

```
pending → validating → running → stopping → stopped
                                         ↘ failed
```

> **Injected environment.** When a session starts, the engine run receives the
> caller's resolved [vault](#vault-env-variables--secrets) environment for the
> session's scope — owner-wide (`global`) entries for a package session, plus
> the target repo's entries (repo overrides global on a key collision) for a
> goal-triggered one. Secret values are injected in memory only: the session
> document persists just a non-secret scope pointer, so a pod failover
> re-resolves the same profile from the vault (picking up any rotated secret),
> and a decrypt failure fails the start rather than running without the secret.
> Platform-reserved keys (`FKST_*`, `GITHUB_TOKEN`, the host allow-list) are
> always dropped. There is no new endpoint — this is automatic.

> **NyxID session identity.** When NyxID is configured, the engine run also
> receives a per-session NyxID identity: at start, fkst-hosted mints one
> non-expiring NyxID agent key on the triggering user's behalf and injects it
> as `NYXID_ACCESS_TOKEN` (plus the `NYXID_URL` origin), so the run acts as that
> user against NyxID. The key is revoked when the session ends; only its
> non-secret id/prefix are persisted (never the full key). This too is automatic
> — there is no new endpoint, and you keep using your normal bearer token.

> **Codex LLM provider.** The engine reasons with `codex`, which fkst-hosted
> points at an LLM provider via a per-session `config.toml` (rendered into a
> private `CODEX_HOME`). By **default** the provider is the NyxID-proxied
> `chrono-llm` service (OpenAI Responses API), authenticated as the session user
> with the injected `NYXID_ACCESS_TOKEN` — so inference runs and is billed as
> that user, with no setup. You can **override** the provider entirely through
> the [vault](#vault-env-variables--secrets) (precedence: raw > structured >
> default), again with no new endpoint:
>
> - **Structured** — set the `variable`s `CODEX_BASE_URL`, `CODEX_MODEL`,
>   `CODEX_WIRE_API` (typically `responses`), and `CODEX_ENV_KEY`, plus a
>   `secret` whose key equals your `CODEX_ENV_KEY` value (the API key codex
>   sends as `Authorization: Bearer`). fkst-hosted renders an
>   OpenAI-compatible provider pointing codex at your endpoint. All four
>   variables must be present, or the default is used.
> - **Raw** — set the `variable` `CODEX_CONFIG_TOML` to a full codex
>   `config.toml`; it is written verbatim. (Your API key still rides the
>   `env_key` named inside it, stored as a separate vault secret.)
>
> The provider API key is never embedded in the rendered config and never
> logged. The chrono-llm default requires the user to have connected
> `chrono-llm` on NyxID (otherwise the start fails `422`). Operators pin the
> default model and proxy route via `FKST_HOSTED_CODEX_MODEL` and
> `FKST_HOSTED_CHRONO_LLM_BASE_URL`.

**Data shape**

```jsonc
// SessionView (GET response)
{
  "id": "f4e2c0a1-…",
  "package_name": "billing-pipeline",
  "package_names": ["billing-pipeline"],   // always ≥ 1 entry
  "status": "running",
  "error": null,                            // populated when status is "failed"
  "owner_user_id": "user-123",
  "org_id": null,
  "goal_id": null,                          // set for goal-triggered sessions
  "repo": null,                             // set for goal-triggered sessions
  "triggered_by": null,                     // e.g. "goal-trigger" / "manual"
  "created_at": "2026-06-15T12:00:00Z",
  "started_at": "2026-06-15T12:00:03Z",
  "stopped_at": null,
  // runtime diagnostics also present: pod_id, fencing_token, pid, runtime_dir
}
```

---

### `POST /api/v1/sessions` — start a session

- **Permission:** **use**-level access to the package — owner, org Member/Admin,
  a `use`-level share, or admin scope. A `read`-only share **cannot** start a
  session.
- **Headers:** `Authorization: Bearer …`, `Content-Type: application/json`.

**Request body**

| Field | Type | Required | Notes |
|-------|------|:--------:|-------|
| `package_name` | string | yes | `^[A-Za-z0-9_-]+$`, ≤ 128 bytes |

**Responses**

| Status | Meaning |
|--------|---------|
| `201 Created` | `{ "id": "<uuid>", "status": "pending" }`; `Location: /api/v1/sessions/<id>` |
| `400` | Invalid package name |
| `403` | No `use` access to the package |
| `404` | Package not found |
| `409` | Another live session already holds this package |

```sh
curl -X POST "$FKST_API/api/v1/sessions" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "package_name": "billing-pipeline" }'
# 201 -> { "id": "f4e2c0a1-…", "status": "pending" }
```

---

### `GET /api/v1/sessions/{id}` — fetch session status

- **Permission:** **Read** on the session — owner, org member (any role), admin
  scope. For goal-triggered sessions the goal's owner can also read.
- **Path parameters:** `id` (UUID).
- **Responses:** `200 OK` with a `SessionView`; `400` malformed id; `404` not
  found / not visible.

```sh
curl -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/sessions/f4e2c0a1-…"
```

---

### `POST /api/v1/sessions/{id}/stop` — request a stop

Asynchronous: returns `202` immediately (for both a real transition and an
idempotent no-op); keep polling `GET` until the status reaches `stopped`.

- **Permission:** **Write** on the session — owner, org Member/Admin, admin scope.
- **Path parameters:** `id` (UUID).
- **Responses:** `202 Accepted` with `{ "status": "stopping" }`; `400` malformed
  id; `403` not permitted; `404` not found.

```sh
curl -X POST -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/sessions/f4e2c0a1-…/stop"
# 202 -> { "status": "stopping" }
```

---

## Goals

A **goal** captures an intent (a prompt), the package(s) to run it with, and an
optional target GitHub repo. You can edit it over time and **trigger** it to
spawn a session. Status lifecycle:

```
not_started → triggered → running → stopped
                                  ↘ failed
```

Packages and the repo can only be changed while the goal is in a **mutable
status**: `not_started`, `stopped`, or `failed`. Title and description are
editable in any status.

**Data shape**

```jsonc
// GoalView
{
  "id": "a1b2…",
  "title": "Build a billing pipeline",
  "description": "Create a billing pipeline that processes invoices.",
  "package_names": ["billing-pipeline"],
  "repo": { "owner": "acme", "name": "billing" },  // or null
  "status": "not_started",
  "owner_user_id": "user-123",
  "org_id": null,
  "active_session_id": null,
  "created_at": "2026-06-15T12:00:00Z",
  "updated_at": "2026-06-15T12:00:00Z"
}
```

---

### `POST /api/v1/goals` — create a goal

- **Permission:** any authenticated caller. With `org_id`, you must be an org
  Member/Admin. Every listed package must be one you can **use**.
- **Headers:** `Authorization: Bearer …`, `Content-Type: application/json`.

**Request body**

| Field | Type | Required | Notes |
|-------|------|:--------:|-------|
| `title` | string | yes | Trimmed, 1–200 characters |
| `description` | string | yes | 1–16384 bytes (the engine-facing prompt) |
| `package_names` | array of string | yes | 1–16 usable packages, no duplicates |
| `repo` | `{ owner, name }` | no | `owner` `^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})$`; `name` `^[A-Za-z0-9._-]{1,100}$` |
| `org_id` | string | no | Attach to an org |

**Responses:** `201 Created` with a `GoalView` and `Location` header; `400`
validation; `403` org membership or package usability; `404`/`400` unknown
package.

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

---

### `GET /api/v1/goals` — list goals

- **Permission:** authenticated. Returns goals you own plus goals in your orgs.
- **Query parameters**

  | Param | Type | Notes |
  |-------|------|-------|
  | `status` | string | Filter by goal status (e.g. `running`) |
  | `limit` | integer | Default `50`, max `200` |
  | `offset` | integer | Default `0` |

- **Responses:** `200 OK` with an array of `GoalView`.

```sh
curl -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/goals?status=not_started&limit=20"
```

---

### `GET /api/v1/goals/{id}` — fetch a goal

- **Permission:** **Read** — owner, org member (any role), admin scope.
- **Path parameters:** `id` (UUID).
- **Responses:** `200 OK` with a `GoalView`; `400` malformed id; `404` not found.

---

### `PATCH /api/v1/goals/{id}` — update a goal

Partial update; absent fields are unchanged.

- **Permission:** **Write** — owner, org Member/Admin, admin scope.
- **Headers:** `Authorization: Bearer …`, `Content-Type: application/json`.

**Request body** (all optional)

| Field | Type | Notes |
|-------|------|-------|
| `title` | string | Editable in any status |
| `description` | string | Editable in any status |
| `package_names` | array of string | Only in a mutable status; same rules as create |
| `repo` | `{ owner, name }` | Only in a mutable status; mutually exclusive with `clear_repo` |
| `clear_repo` | boolean | `true` removes the repo; mutually exclusive with `repo` |

**Responses:** `200 OK` with the updated `GoalView`; `400` validation / both
`repo` and `clear_repo`; `403` not permitted; `404` not found; `409` the change
touches packages/repo while the goal is not in a mutable status.

```sh
curl -X PATCH "$FKST_API/api/v1/goals/a1b2…" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "title": "Build the billing pipeline (v2)" }'
```

---

### `DELETE /api/v1/goals/{id}` — delete a goal

- **Permission:** **Manage** — owner, org Admin, admin scope.
- **Responses:** `204 No Content`; `403` not permitted; `404` not found; `409`
  the goal is not in a mutable status (stop it first).

---

### `POST /api/v1/goals/{id}/trigger` — trigger a goal

Spawns a new session for the goal against a GitHub repository.

- **Permission:** the goal **owner**, an org **Member/Admin** (not Viewer) for
  org goals, or admin scope.
- **Headers:** `Authorization: Bearer …`, `Content-Type: application/json`.
- **Requires:** the fkst-hosted GitHub App installed on the target repo (else
  `422`). `create_new` mode also requires NyxID's credential proxy.

**Request body**

| Field | Type | Required | Notes |
|-------|------|:--------:|-------|
| `repo_mode` | `"existing"` \| `"create_new"` | no | Defaults to `existing` |
| `repo` | `{ owner, name }` | no | **existing** mode only — overrides the goal's stored repo for this run |
| `create` | `CreateRepoSpec` | for `create_new` | Required in `create_new` mode; forbidden in `existing` mode |

`CreateRepoSpec`:

| Field | Type | Required | Notes |
|-------|------|:--------:|-------|
| `name` | string | yes | New repository name |
| `private` | boolean | no | Defaults to `true` |
| `description` | string | no | |
| `org_login` | string | no | Create under this org; otherwise under the authenticated user |

**Responses**

`202 Accepted`:

```json
{ "goal_id": "a1b2…", "session_id": "f4e2c0a1-…", "goal_status": "triggered", "session_status": "pending" }
```

| Status | Meaning |
|--------|---------|
| `202` | Triggered — poll the returned `session_id` |
| `400` | Invalid `repo_mode`/`create` combination, or invalid repo shape |
| `403` | Not permitted, or a listed package is no longer usable |
| `404` | Goal not found |
| `409` | The goal is already triggered or running |
| `422` | No repo to use, package missing, or the GitHub App is not installed |

```sh
# Trigger against the stored (or overridden) repo
curl -X POST "$FKST_API/api/v1/goals/a1b2…/trigger" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "repo": { "owner": "acme", "name": "billing" } }'

# Create a brand-new repo, then trigger against it
curl -X POST "$FKST_API/api/v1/goals/a1b2…/trigger" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "repo_mode": "create_new", "create": { "name": "new-billing-repo", "private": true, "org_login": "acme" } }'
```

---

## GitHub issues hub

Read and manage GitHub issues across **all** of your linked GitHub accounts.
GitHub is reached only through NyxID's credential-injection proxy (RFC 8693
delegation) using **your** OAuth grant — fkst-hosted never holds a GitHub token.
All endpoints require authentication; issue bodies and tokens are never logged.

**Account selection (single-target operations):** the optional `account` field
(a linked GitHub login) chooses which linked account to act under. It is
**implied** when exactly one account is linked; when several are linked it is
**required** — an absent or unknown `account` returns `422`.

**Upstream status mapping (single-target operations):** GitHub `404` → `404`;
`401`/`403` without rate-limit evidence → `403`; `422` → `422` (surfacing
GitHub's first error message); `403`/`429` with rate-limit evidence → `429` with
a `Retry-After` header; any other `5xx` → `502 upstream_error`.

**Data shapes**

```jsonc
// AccountView
{ "connection_id": "c1", "login": "octocat", "primary": true }

// IssueView ("body" is populated only on a single-issue GET; null in lists)
{
  "account": "octocat", "repository": "acme/billing", "number": 7, "id": 1001,
  "title": "Fix the thing", "body": null, "state": "open",
  "labels": ["bug"], "assignees": ["octocat"], "comments": 3,
  "html_url": "https://github.com/acme/billing/issues/7",
  "created_at": "2026-06-15T12:00:00Z", "updated_at": "2026-06-15T12:00:00Z"
}

// CommentView
{ "id": 55, "user": "octocat", "body": "looks good",
  "html_url": "https://github.com/acme/billing/issues/7#issuecomment-55",
  "created_at": "…", "updated_at": "…" }
```

---

### `GET /api/v1/github/accounts` — list linked accounts

- **Permission:** authenticated.
- **Responses:** `200 OK` with an array of `AccountView`; `503` if the credential
  proxy is unavailable.

```sh
curl -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/github/accounts"
```

---

### `GET /api/v1/github/issues` — aggregate issues across accounts

Queries each linked account concurrently and merges the results. **Always `200`**
once your accounts resolve — a slow/failing/rate-limited account is reported in
its own `error` block instead of failing the whole request. Zero linked accounts
yields `{ "results": [] }`.

**Query parameters**

| Param | Default | Notes |
|-------|---------|-------|
| `accounts` | all | Comma-separated logins to restrict the fan-out to |
| `filter` | `assigned` | GitHub issue filter |
| `state` | `open` | `open` / `closed` / `all` |
| `labels` | — | Comma-separated label names |
| `page` | `1` | |
| `per_page` | `30` | Clamped to `1..=50` |

**Response** (`200 OK`):

```json
{ "results": [
  { "account": "octocat", "issues": [ /* IssueView, body suppressed */ ],
    "page": 1, "per_page": 30, "has_more": true,
    "rate_limit": { "remaining": 4998, "reset_epoch": 1700000000 } },
  { "account": "hubber", "issues": [], "page": 1, "per_page": 30, "has_more": false,
    "error": { "kind": "rate_limited", "message": "github rate limited; retry later", "retry_after_secs": 41 } }
] }
```

Per-account `error.kind` is one of `rate_limited` | `auth` | `upstream` |
`network`. Only a delegation / account-listing failure bubbles up as `503`.

```sh
curl -H "Authorization: Bearer $TOKEN" \
  "$FKST_API/api/v1/github/issues?filter=assigned&state=open&per_page=50"
```

---

### `POST /api/v1/github/repos/{owner}/{repo}/issues` — create an issue

- **Permission:** authenticated (acts under your linked GitHub account).
- **Headers:** `Authorization: Bearer …`, `Content-Type: application/json`.
- **Path parameters:** `owner`, `repo`.

**Request body**

| Field | Type | Required | Notes |
|-------|------|:--------:|-------|
| `title` | string | yes | Must not be empty |
| `body` | string | no | |
| `labels` | array of string | no | |
| `assignees` | array of string | no | |
| `account` | string | conditionally | Required when several accounts are linked |

**Responses:** `201 Created` with the created `IssueView`. On success the
response copies GitHub's `x-ratelimit-remaining` and `x-ratelimit-reset` headers
so you can pace writes. `400` empty title; `422` account-selection or GitHub
validation; `429` rate-limited; `502` upstream error.

```sh
curl -X POST "$FKST_API/api/v1/github/repos/acme/billing/issues" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "title": "Invoices double-counted", "body": "Steps to reproduce…", "labels": ["bug"] }'
```

---

### `GET /api/v1/github/repos/{owner}/{repo}/issues/{number}` — fetch one issue

- **Permission:** authenticated.
- **Path parameters:** `owner`, `repo`, `number` (positive integer).
- **Query parameters:** `account` (required when several accounts are linked).
- **Responses:** `200 OK` with an `IssueView` (with `body` populated); `404`;
  `422`; `429`; `502`.

```sh
curl -H "Authorization: Bearer $TOKEN" \
  "$FKST_API/api/v1/github/repos/acme/billing/issues/7"
```

---

### `PATCH /api/v1/github/repos/{owner}/{repo}/issues/{number}` — update an issue

Only the fields you supply are changed.

- **Permission:** authenticated.
- **Headers:** `Authorization: Bearer …`, `Content-Type: application/json`.

**Request body** (all optional): `title`, `body`, `state` (e.g. `open`/`closed`),
`labels`, `assignees`, `account`.

**Responses:** `200 OK` with the updated `IssueView`; `404`; `422`; `429`; `502`.

```sh
curl -X PATCH "$FKST_API/api/v1/github/repos/acme/billing/issues/7" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "state": "closed" }'
```

---

### `GET /api/v1/github/repos/{owner}/{repo}/issues/{number}/comments` — list comments

- **Permission:** authenticated.
- **Query parameters:** `account` (when several accounts linked), `page`
  (default `1`), `per_page` (default `30`).
- **Responses:** `200 OK` with an array of `CommentView`.

```sh
curl -H "Authorization: Bearer $TOKEN" \
  "$FKST_API/api/v1/github/repos/acme/billing/issues/7/comments"
```

---

### `POST /api/v1/github/repos/{owner}/{repo}/issues/{number}/comments` — add a comment

- **Permission:** authenticated.
- **Headers:** `Authorization: Bearer …`, `Content-Type: application/json`.

**Request body**

| Field | Type | Required | Notes |
|-------|------|:--------:|-------|
| `body` | string | yes | Must not be empty |
| `account` | string | conditionally | Required when several accounts are linked |

**Responses:** `201 Created` with the created `CommentView`; `400` empty body;
`422`; `429`; `502`.

```sh
curl -X POST "$FKST_API/api/v1/github/repos/acme/billing/issues/7/comments" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "body": "Thanks, looking into this." }'
```

---

## Vault (env variables & secrets)

The **vault** is a fkst-hosted-owned, encrypted-at-rest key–value store for the
environment **variables** (non-secret config) and **secrets** an engine run
needs. Each entry has a `kind`, an env-var `key`, and a `scope` — either
owner-wide (`global`) or a specific GitHub repo. Entries are owned by the caller
and enforced by the same owner/org authorization as the rest of the API.

> **Secrets are write-only.** A `secret` value is accepted on `PUT` and is
> **never** returned by any read — not in the `PUT` response and not in `GET`.
> A read shows only a display-only `masked_hint` (`"…last4"`). A `variable`
> value, being non-secret config, **is** returned. Secret values are
> envelope-encrypted (AES-256-GCM) at rest and never stored in plaintext.

**Common data shapes**

```jsonc
// Scope (request + response): exactly one of global / repo
{ "global": true }                 // owner-wide
{ "repo": "acme/site" }            // a specific repo

// EntryView (response): a secret omits `value`; a variable includes it
{
  "id": "f0e1d2c3-…",
  "key": "OPENAI_API_KEY",
  "kind": "secret",                // or "variable"
  "scope": { "global": true },
  "masked_hint": "…cret",          // secrets only; display-only
  "value": "debug",                // variables only; omitted for secrets
  "updated_at": "2026-06-16T12:00:00Z"
}
```

See [vault limits](#vault-limits) for the value-size and per-scope caps.

---

### `GET /api/v1/vault/entries` — list entries in a scope

Returns the caller's entries in a scope (key-sorted). **Secret values are never
included**; variable values are.

- **Permission:** authenticated; returns only your own entries.
- **Headers:** `Authorization: Bearer …`.

**Query parameters**

| Param | Values | Notes |
|-------|--------|-------|
| `scope` | `global` (default), `repo` | The scope to list |
| `repo` | `owner/name` | Required when `scope=repo` |

```sh
curl -H "Authorization: Bearer $TOKEN" \
  "$FKST_API/api/v1/vault/entries?scope=global"
# [ { "id": "...", "key": "OPENAI_API_KEY", "kind": "secret",
#     "scope": { "global": true }, "masked_hint": "…cret",
#     "updated_at": "2026-06-16T12:00:00Z" } ]
```

---

### `PUT /api/v1/vault/entries` — create or update an entry

Upsert an entry by `(owner, scope, key)`. A `secret` value is encrypted and
stored with a masked hint; a `variable` value is stored as plaintext config.

- **Permission:** authenticated. If `org_id` is supplied, the caller must be an
  **org Member or Admin** (else `403`); the entry is still owned by the caller.
- **Headers:** `Authorization: Bearer …`, `Content-Type: application/json`.

**Request body**

| Field | Type | Required | Notes |
|-------|------|:--------:|-------|
| `scope` | object | yes | Exactly one of `{ "global": true }` or `{ "repo": "owner/name" }` |
| `key` | string | yes | Env-var name; must match `^[A-Za-z_][A-Za-z0-9_]*$` |
| `kind` | string | yes | `secret` or `variable` |
| `value` | string | yes | ≤ the value-size cap; a `secret` is encrypted at rest |
| `org_id` | string | no | Attach to an org (caller must be a writer) |

**Responses**

| Status | Meaning |
|--------|---------|
| `200 OK` | Upserted. Body is the redacted `EntryView` (no secret value) |
| `400` | Malformed body or scope (not exactly one of global/repo) |
| `403` | `org_id` given but caller is not an org writer |
| `422` | Invalid key name, a **reserved** platform key (`FKST_*`, `GITHUB_TOKEN`, allow-listed host vars), an oversized value, or the per-scope entry cap exceeded |
| `500` | Vault key provider not configured (a deploy-time misconfiguration) |

```sh
curl -X PUT "$FKST_API/api/v1/vault/entries" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{
    "scope": { "global": true },
    "key": "OPENAI_API_KEY",
    "kind": "secret",
    "value": "sk-…"
  }'
# 200 -> { "id": "...", "key": "OPENAI_API_KEY", "kind": "secret",
#          "scope": { "global": true }, "masked_hint": "…",
#          "updated_at": "..." }   // note: no `value`
```

---

### `DELETE /api/v1/vault/entries/{id}` — delete an entry

Remove an entry by its UUID.

- **Permission:** authenticated; you must own (or be an org admin of) the entry.
- **Headers:** `Authorization: Bearer …`.

**Responses**

| Status | Meaning |
|--------|---------|
| `204 No Content` | Deleted |
| `400` | `{id}` is not a valid UUID |
| `403` | The entry exists but you cannot manage it |
| `404` | No such entry |

```sh
curl -X DELETE "$FKST_API/api/v1/vault/entries/f0e1d2c3-…" \
  -H "Authorization: Bearer $TOKEN"
# 204 No Content
```

---

## Appendix: data types & limits

### Enumerations

| Type | Values (wire form) |
|------|--------------------|
| Session status | `pending`, `validating`, `running`, `stopping`, `stopped`, `failed` |
| Goal status | `not_started`, `triggered`, `running`, `stopped`, `failed` |
| Share grantee kind | `user`, `org` |
| Share level | `read`, `use` |
| Conformance status | `ok`, `failed`, `skipped` |
| GitHub repo mode (goal trigger) | `existing`, `create_new` |
| Vault entry kind | `variable`, `secret` |

### Package limits

| Limit | Value |
|-------|-------|
| Package name | `^[A-Za-z0-9_-]+$` |
| Files per package | 256 |
| Single file path length | 512 bytes |
| Single file content | 1 MiB |
| Total content | 12 MiB |
| `composed_deps` entries | 256, each ≤ 256 bytes (no newline/NUL) |
| File paths | forward-slash only; no absolute paths, `..`, backslash, or control chars; all content UTF-8 |
| Engine entry (required) | at least one `departments/<name>/main.lua` or `raisers/<name>.lua` |

### Goal limits

| Limit | Value |
|-------|-------|
| Title | 1–200 characters |
| Description | 1–16384 bytes |
| Packages per goal | 1–16 |
| Repo owner | `^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})$` |
| Repo name | `^[A-Za-z0-9._-]{1,100}$` |

### Generation limits

| Limit | Value |
|-------|-------|
| Description | 1–8192 bytes |
| Generated name (when given) | `^[A-Za-z0-9_-]+$` |

### Vault limits

| Limit | Value |
|-------|-------|
| Entry key | `^[A-Za-z_][A-Za-z0-9_]*$` |
| Reserved keys (rejected with `422`) | any `FKST_*`, `GITHUB_TOKEN`, `FKST_GITHUB_TOKEN_FILE`, `FKST_GOAL_FILE`, and the allow-listed host vars (`PATH`, `HOME`, `CODEX_HOME`, `LANG`, `LC_ALL`, `TMPDIR`, `TZ`, `SSL_CERT_FILE`, `SSL_CERT_DIR`) |
| Single value | ≤ 64 KiB (default; `FKST_HOSTED_VAULT_VALUE_BYTE_CAP`) |
| Entries per scope | ≤ 100 (default; `FKST_HOSTED_VAULT_ENTRIES_PER_SCOPE_CAP`) |
| Scope | `global` (owner-wide) or `repo:<owner>/<name>` |
| Secret encryption | AES-256-GCM envelope (per-secret DEK wrapped by a KEK); secrets never returned, logged, or stored in plaintext |
