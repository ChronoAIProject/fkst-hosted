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
- [Inline secrets & variables](#inline-secrets--variables)
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

### Authentication — you authenticate to NyxID, not to fkst-hosted

fkst-hosted is deployed as a **NyxID downstream service**: it has **no public
ingress** (the backend is a cluster-internal `ClusterIP` service) and is reached
**only through the NyxID proxy**. You authenticate to **NyxID**, and NyxID
forwards your verified identity to fkst-hosted.

So the bearer token you send is a **client ↔ NyxID-proxy** concern:

```
Authorization: Bearer <your-nyxid-token>
```

fkst-hosted itself does **not** verify your token, fetch any JWKS, or check any
signature — the proxy already authenticated you. It simply **trusts** the
identity the proxy injects into the forwarded request and authorizes from there.

> **Internal headers (not part of the public contract).** Between the proxy and
> fkst-hosted, identity travels in `X-NyxID-*` request headers: a (decode-only)
> `X-NyxID-Identity-Token` JWT carrying `sub`, `permissions[]`, `roles[]`,
> `groups[]`, with an `X-NyxID-User-Id` (+ `-Email` / `-Name`) fallback. Clients
> never set these — the proxy does. They are documented here only to explain the
> trust model; do not depend on them.

If a request reaches fkst-hosted with **no** injected identity at all (only
possible when the proxy is misconfigured or bypassed), it returns
`401 Unauthorized`.

### Authorization model — two layers

Every protected endpoint is gated by **two** independent layers; **both** must
pass.

**Layer 1 — permission (the action layer).** fkst-hosted defines a vocabulary of
`fkst:*` permission strings and requires the matching one for each endpoint.
**NyxID owns the assignment** of these permissions to users/roles; they arrive
in the injected identity token's `permissions[]`. fkst-hosted does **not** keep a
local role→permission matrix or role store — it only checks exact-string
membership. A caller whose permission set lacks the required string gets
`403 forbidden`. The permission `fkst:admin` is the platform escape hatch and
bypasses both layers.

| Permission | Gates |
|------------|-------|
| `fkst:session:read` | `GET /sessions/{id}` |
| `fkst:session:stop` | `POST /sessions/{id}/stop` |
| `fkst:goal:read` | `GET /goals`, `GET /goals/{id}` |
| `fkst:goal:create` | `POST /goals` |
| `fkst:goal:update` | `PATCH /goals/{id}` |
| `fkst:goal:delete` | `DELETE /goals/{id}` |
| `fkst:goal:trigger` | `POST /goals/{id}/trigger` |
| `fkst:github:read` | `GET /github/accounts`, `/github/issues`, issue/comment reads |
| `fkst:github:write` | `POST`/`PATCH` of issues and comments |
| `fkst:catalog:read` | all `GET /catalog/*` |
| `fkst:repo:setup` | `POST /repos/{owner}/{name}/fkst-setup` |
| `fkst:admin` | everything (bypass) |

**Layer 2 — ownership / org (the object layer).** After the permission check,
access to the **specific** resource is decided in this order:

1. **Admin** — a caller with the `fkst:admin` permission may act on any resource.
2. **Owner** — the resource's `owner_user_id` may act on their own resource.
3. **Organization role** — when the resource has an `org_id`, the caller's role
   in that org (resolved live via NyxID) grants:

   | Org role | Read | Write | Manage |
   |----------|:----:|:-----:|:------:|
   | Viewer | ✅ | — | — |
   | Member | ✅ | ✅ | — |
   | Admin | ✅ | ✅ | ✅ |

4. **Legacy** — resources with no `owner_user_id` are open to any caller that
   cleared the permission layer.

> Org-role lookups use the caller's forwarded token and **fail soft** when it is
> absent: read visibility degrades to the caller's own resources.

> **Operator note.** Because the action permission now comes from NyxID, NyxID
> must assign the matching `fkst:*` permissions to org roles to preserve the
> table above: org **Admin** → all `fkst:*`, **Member** → read + write + trigger,
> **Viewer** → read.

> **Ownership is database-free.** The `owner_user_id` and `org_id` above are
> sourced with no application datastore: for **sessions** from the controller's
> in-memory session state, and for **goals** from the in-memory goal store
> (mirrored to a server-controlled hidden marker on the goal's GitHub Issue, which
> is re-validated server-side and never trusted from a client-editable region).
> The owner is your **NyxID subject (`sub`)** — the same identity the action layer
> uses — **not** a GitHub login, so no subject↔GitHub-login binding is involved in
> authorization. Org-scoped list visibility is filtered in memory against the
> orgs your forwarded token resolves to (rule 4 above never applies to a goal: an
> issue-backed goal always carries an owner).

The three object actions map to endpoints as:

- **Read** — fetching a resource.
- **Write** — updating a resource.
- **Manage** — deleting a resource.

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
| `401` | `unauthorized` | No proxy-injected identity reached the service (proxy misconfigured/bypassed) | |
| `403` | `forbidden` | Authenticated but missing the required `fkst:*` permission, or not permitted on the specific resource | |
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

## Sessions

A **session** runs your repo's packages on the fkst engine. Sessions are created
exclusively by **triggering a goal** (`POST /api/v1/goals/{id}/trigger`): the
trigger clones the goal's repo, loads the named packages from
`<repo>/.fkst/packages/`, and starts the engine. You then poll the session's
status and stop it through the endpoints below. Status lifecycle:

```
pending → validating → running → stopping → stopped
                                         ↘ failed
```

> **Injected environment.** When a session starts, the engine run receives the
> caller's resolved [inline-secret](#inline-secrets--variables) environment for
> the session's scope — owner-wide (`global`) entries overlaid by the target
> repo's entries (repo overrides global on a key collision). Values are supplied
> inline at goal trigger and injected in memory only: the session document
> persists just a non-secret scope pointer, so a same-process failover re-injects
> the controller-held values (a controller **restart** loses them — re-trigger to
> re-supply). Platform-reserved keys (`FKST_*`, `GITHUB_TOKEN`, the host
> allow-list) are always dropped. There is no separate endpoint — the secrets
> ride the goal-trigger body.

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
> the goal-trigger [`secrets`](#post-apiv1goalsidtrigger--trigger-a-goal) array
> (precedence: raw > structured > default), again with no separate endpoint:
>
> - **Structured** — supply the inline `variable`s `CODEX_BASE_URL`,
>   `CODEX_MODEL`, `CODEX_WIRE_API` (typically `responses`), and `CODEX_ENV_KEY`,
>   plus an inline `secret` whose key equals your `CODEX_ENV_KEY` value (the API
>   key codex sends as `Authorization: Bearer`). fkst-hosted renders an
>   OpenAI-compatible provider pointing codex at your endpoint. All four
>   variables must be present, or the default is used.
> - **Raw** — supply the inline `variable` `CODEX_CONFIG_TOML` as a full codex
>   `config.toml`; it is written verbatim. (Your API key still rides the
>   `env_key` named inside it, supplied as a separate inline secret.)
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

### Pinning Ornn skills

`POST /api/v1/goals/{id}/trigger` accepts an optional `ornn_skills` array. Each
pin is a concrete `{ kind, name, version }`:

| Field | Type | Notes |
|-------|------|-------|
| `kind` | string | `"skill"` or `"skillset"` |
| `name` | string | `^[a-z0-9][a-z0-9-]*$`, ≤ 64 bytes |
| `version` | string | `<major>.<minor>` (no leading zeros, no patch, no `@latest`) |

At session start fkst-hosted fetches each pinned skill **as you** — through
NyxID's `ornn-api` proxy, so Ornn enforces your private/shared/public/system
visibility — and installs it into the session's private codex home so the run's
`codex` can invoke it. A **skillset** is expanded to its closure (every member
skill is installed) and its master prompt is added to the session's
`AGENTS.md`. Pinning the same skill at two different versions (directly, or via
a skillset member) is rejected (`422`); a missing or forbidden pin makes the
session start **fail** rather than silently dropping it. Browse what you can
pin via the [catalog API](#skill-catalog-ornn). The picker UI should
pre-validate the version-conflict before triggering.

---

### `GET /api/v1/sessions/{id}` — fetch session status

- **Permission:** `fkst:session:read`, then **Read** on the session — owner, org
  member (any role), or the `fkst:admin` permission. For goal-triggered sessions
  the goal's owner can also read.
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

- **Permission:** `fkst:session:stop`, then **Write** on the session — owner, org
  Member/Admin, or the `fkst:admin` permission.
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

> **Durable representation.** A goal with a target repo is mirrored as a
> **GitHub Issue** on that repo, labelled `fkst-hosted:goal` and
> `status:<status>` (the label tracks the lifecycle above). The issue body
> carries only a non-sensitive summary (title, package count, repo slug) — the
> **prompt/description is never written to GitHub**; it lives only in controller
> memory, which is authoritative for reads. A goal created without a repo is
> held in memory until a repo is set (at trigger time), which materialises the
> issue. Deleting a goal **closes** its issue (GitHub issues cannot be deleted).

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

- **Permission:** `fkst:goal:create`. With `org_id`, you must be an org
  Member/Admin. Every listed package must be one you can **use**.
- **Headers:** `Content-Type: application/json` (the bearer goes to the NyxID
  proxy, not to fkst-hosted directly).

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

- **Permission:** `fkst:goal:read`. Returns goals you own plus goals in your orgs.
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

- **Permission:** `fkst:goal:read`, then **Read** — owner, org member (any role),
  or the `fkst:admin` permission.
- **Path parameters:** `id` (UUID).
- **Responses:** `200 OK` with a `GoalView`; `400` malformed id; `404` not found.

---

### `PATCH /api/v1/goals/{id}` — update a goal

Partial update; absent fields are unchanged.

- **Permission:** `fkst:goal:update`, then **Write** — owner, org Member/Admin,
  or the `fkst:admin` permission.
- **Headers:** `Content-Type: application/json`.

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

- **Permission:** `fkst:goal:delete`, then **Manage** — owner, org Admin, or the
  `fkst:admin` permission.
- **Responses:** `204 No Content`; `403` not permitted; `404` not found; `409`
  the goal is not in a mutable status (stop it first).

---

### `POST /api/v1/goals/{id}/trigger` — trigger a goal

Spawns a new session for the goal against a GitHub repository.

- **Permission:** `fkst:goal:trigger`, then the goal **owner**, an org
  **Member/Admin** (not Viewer) for org goals, or the `fkst:admin` permission.
- **Headers:** `Content-Type: application/json`.
- **Requires:** the fkst-hosted GitHub App installed on the target repo (else
  `422`). `create_new` mode also requires NyxID's credential proxy.

**Request body**

| Field | Type | Required | Notes |
|-------|------|:--------:|-------|
| `repo_mode` | `"existing"` \| `"create_new"` | no | Defaults to `existing` |
| `repo` | `{ owner, name }` | no | **existing** mode only — overrides the goal's stored repo for this run |
| `create` | `CreateRepoSpec` | for `create_new` | Required in `create_new` mode; forbidden in `existing` mode |
| `ornn_skills` | array | no | Ornn skills/skillsets to inject — see [Pinning Ornn skills](#pinning-ornn-skills) |
| `secrets` | array of `InlineSecret` | no | [Inline secrets/variables](#inline-secrets--variables) injected into this run's engine env (in memory only). See limits |

`CreateRepoSpec`:

| Field | Type | Required | Notes |
|-------|------|:--------:|-------|
| `name` | string | yes | New repository name |
| `private` | boolean | no | Defaults to `true` |
| `description` | string | no | |
| `org_login` | string | no | Create under this org; otherwise under the authenticated user |

`InlineSecret`:

| Field | Type | Required | Notes |
|-------|------|:--------:|-------|
| `key` | string | yes | Env-var name; `^[A-Za-z_][A-Za-z0-9_]*$`; not a [reserved key](#inline-secret-limits) |
| `value` | string | yes | ≤ the value-size cap; never logged or returned |
| `kind` | `"secret"` \| `"variable"` | no | Defaults to `secret` |

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
| `422` | No repo to use, package missing, the GitHub App is not installed, or an inline secret has a reserved/invalid/oversized key |

```sh
# Trigger against the stored (or overridden) repo, with an inline secret
curl -X POST "$FKST_API/api/v1/goals/a1b2…/trigger" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "repo": { "owner": "acme", "name": "billing" },
        "secrets": [ { "key": "OPENAI_API_KEY", "value": "sk-…" } ] }'

# Create a brand-new repo, then trigger against it
curl -X POST "$FKST_API/api/v1/goals/a1b2…/trigger" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "repo_mode": "create_new", "create": { "name": "new-billing-repo", "private": true, "org_login": "acme" } }'
```

---

## The `fkst-goal` issue template & parse contract

A user can declare a goal by opening a GitHub issue from the **fkst goal** form
(`.github/ISSUE_TEMPLATE/fkst-goal.yml`) instead of (or in addition to) the
inline [trigger](#post-apiv1goalsidtrigger--trigger-a-goal) payload. This section
defines the **canonical, machine-parseable contract** the backend keys on to
extract the Goal, the package-name list, and the Ornn skill pins from a
user-authored issue body. (The form ships here; the submit endpoint that consumes
this contract — and the `fkst-goal` label — are sibling milestone-#5 issues.)

The form has a stable title prefix `[fkst-goal]: ` and renders three sections as
`### ` headings. The parser splits the body on the literal headings — **`### Goal`**,
**`### Package Name List`**, **`### Ornn Skill List`** — and a section's content is
every line after its heading up to the next `### ` heading (or end of body),
trimmed. (The form's field labels are these exact strings; renaming a label
without updating this contract breaks parsing.)

| Section | Required | Rule |
|---------|----------|------|
| `### Goal` | yes | The entire trimmed text block, verbatim, becomes `GoalDoc.description` (the engine prompt). It is **secret downstream** — never echoed back into the server-rendered marker or summary. |
| `### Package Name List` | yes | Split on newlines; trim; drop empty lines. Each remaining line is one package name, validated exactly as `validate_goal_fields` / `resolve_package_roots`: `^[A-Za-z0-9_-]+$`, count `1..=16`, no duplicates, `host` reserved. |
| `### Ornn Skill List` | no | Split on newlines; trim; drop empty lines. Each remaining line must match `^(skill\|skillset):([a-z0-9][a-z0-9-]*)@(0\|[1-9][0-9]*)\.(0\|[1-9][0-9]*)$`, mapping to `OrnnSkillPin{kind, name, version}`. Validation defers to `validate_pins` (name ≤ 64 bytes, `major.minor` only, same-name conflicting versions reject). An empty/absent section means zero pins. |

**Malformed → `422 Unprocessable Entity`** (enforced by the sibling submit
endpoint): a missing `### Goal` section or an empty Goal after trim; a
missing/empty `### Package Name List`; any package line failing the
regex/count/duplicate/reserved rule; any Ornn line not matching the pin grammar;
or a duplicate `### ` heading. The response carries a field-scoped error so the
author can self-correct.

> ⚠️ Secrets, tokens, and environment variables are **never** read from the
> issue — they are supplied only through the trigger API.

### Worked example

Issue body:

```
### Goal

Build a CLI that summarizes a git repo's commit history into a weekly digest.

### Package Name List

repo-reader
digest-writer

### Ornn Skill List

skill:git-log@1.0
skillset:report-templates@2.3
```

Parsed result:

- `description` = `"Build a CLI that summarizes a git repo's commit history into a weekly digest."`
- `package_names` = `["repo-reader", "digest-writer"]`
- pins = `[{kind: skill, name: "git-log", version: "1.0"}, {kind: skillset, name: "report-templates", version: "2.3"}]`

---

## Skill catalog (Ornn)

Browse the Ornn **skills / skillsets** you may attach to a session via
[`ornn_skills`](#pinning-ornn-skills), and list their concrete versions for a
picker. Every call forwards **your** NyxID token to Ornn through the credential
proxy, so the results honor your private / shared / public / system visibility —
beyond the `fkst:catalog:read` action gate, **fkst-hosted applies no object
permission logic of its own**; Ornn's result (including any `4xx`/`5xx`) is passed
through as the authoritative answer. When NyxID is not configured these endpoints
answer `503`. All endpoints require the `fkst:catalog:read` permission.

### `GET /api/v1/catalog/skills` · `GET /api/v1/catalog/skillsets`

List skills (or skillsets) visible to you in a scope.

**Query parameters**

| Param | Type | Notes |
|-------|------|-------|
| `scope` | `mine` \| `shared` \| `public` | Defaults to `public` |
| `system` | `any` \| `only` \| `exclude` | **skills only** — filter by the system flag (default `any`); ignored for skillsets |
| `kind` | string | Optional Ornn kind filter |
| `tags` | string | Optional comma-separated tag filter |
| `q` | string | Optional free-text query |
| `page` | string | Optional page number |

**Response** `200 OK` — the requested collection is populated, the other is
omitted:

```jsonc
{
  "data": {
    "skills": [
      { "name": "code-format", "guid": "…", "description": "…",
        "tags": ["lint"], "is_private": false, "is_system_skill": false }
    ],
    "page": 1, "page_size": 20, "total": 1
  }
}
```

| Status | Meaning |
|--------|---------|
| `200` | Listing (Ornn's result, relayed) |
| `400` | Invalid `scope` / `system` value |
| `401`/`403`/`404`/`429` | Relayed verbatim from Ornn |
| `503` | NyxID/Ornn not configured |

```sh
# Your own private skills
curl -H "Authorization: Bearer $TOKEN" \
  "$FKST_API/api/v1/catalog/skills?scope=mine"
# Public system skills only
curl -H "Authorization: Bearer $TOKEN" \
  "$FKST_API/api/v1/catalog/skills?scope=public&system=only"
```

### `GET /api/v1/catalog/skills/{name}/versions` · `GET /api/v1/catalog/skillsets/{name}/versions`

List a skill's (or skillset's) versions, newest-first — load lazily for the
picker once a row is selected.

```jsonc
// 200
{ "data": { "name": "code-format",
            "versions": [ { "version": "2.0", "is_deprecated": false },
                          { "version": "1.0", "is_deprecated": true } ] } }
```

- **Path parameters:** `name` (`^[a-z0-9][a-z0-9-]*$`).
- **Responses:** `200`; `400` malformed name; `401`/`403`/`404` relayed from
  Ornn; `503` NyxID/Ornn not configured.

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

- **Permission:** `fkst:github:read`.
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

- **Permission:** `fkst:github:read`.

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

- **Permission:** `fkst:github:write` (acts under your linked GitHub account).
- **Headers:** `Content-Type: application/json`.
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

- **Permission:** `fkst:github:read`.
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

- **Permission:** `fkst:github:write`.
- **Headers:** `Content-Type: application/json`.

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

- **Permission:** `fkst:github:read`.
- **Query parameters:** `account` (when several accounts linked), `page`
  (default `1`), `per_page` (default `30`).
- **Responses:** `200 OK` with an array of `CommentView`.

```sh
curl -H "Authorization: Bearer $TOKEN" \
  "$FKST_API/api/v1/github/repos/acme/billing/issues/7/comments"
```

---

### `POST /api/v1/github/repos/{owner}/{repo}/issues/{number}/comments` — add a comment

- **Permission:** `fkst:github:write`.
- **Headers:** `Content-Type: application/json`.

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

## Repo fkst-context setup

### `POST /api/v1/repos/{owner}/{name}/fkst-setup` — initialize a repo for fkst

Scaffolds a `.fkst/` directory onto the repository's **default branch** so the
repo is ready to drive fkst sessions. It commits, as one atomic commit
(`fkst: initialize fkst context`):

- `.fkst/packages/example/departments/example/main.lua` — a minimal valid example
  package (passes the engine's package validation),
- `.fkst/packages/example/README.md` — explains the example package,
- `.fkst/AGENTS.md` — the per-repo base instruction file prepended to every fkst
  session spawned from this repo.

- **Permission:** `fkst:repo:setup`. With `org_id`, you must be an org
  Member/Admin. The repository's GitHub ownership is enforced by the **fkst-hosted
  GitHub App installation** — the App must already be installed on `{owner}/{name}`
  (it holds the `contents:write` used to commit). Unlike the issues hub, this
  endpoint writes through the **App installation token**, not your linked GitHub
  account (which is read-only).
- **Headers:** `Content-Type: application/json` only when sending a body.

**Path parameters**

| Param | Notes |
|-------|-------|
| `owner` | `^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})$` |
| `name` | `^[A-Za-z0-9._-]{1,100}$` |

**Query parameters**

| Param | Type | Notes |
|-------|------|-------|
| `force` | boolean | Re-commit the three scaffold paths over an existing `.fkst` (never deletes other `.fkst` content). Default `false` = safe, no-overwrite. |

**Request body** (optional)

| Field | Type | Required | Notes |
|-------|------|:--------:|-------|
| `org_id` | string | no | Enforce an org-writer check for this org |

**Idempotency.** Before writing, the endpoint checks whether `.fkst` already
exists. If it does and `force` is not set, it returns **`200`** with
`already_initialized: true` and `created_paths: []` — **no** content is
overwritten. A fresh repo is scaffolded and returns **`201`**; a `?force=true`
re-commit over an existing `.fkst` returns **`200`** with
`already_initialized: false`.

**Response body**

```json
{
  "repo": { "owner": "acme", "name": "site" },
  "default_branch": "main",
  "commit_sha": "abc123...",
  "created_paths": [
    ".fkst/packages/example/departments/example/main.lua",
    ".fkst/packages/example/README.md",
    ".fkst/AGENTS.md"
  ],
  "already_initialized": false
}
```

On an already-initialized no-op, `default_branch` and `commit_sha` are `null` and
`created_paths` is empty.

**Responses:** `201` freshly scaffolded; `200` already initialized (no-op) or
forced re-commit; `400` malformed owner/name; `403` missing `fkst:repo:setup` or
failed org-writer check; `404` repository not found; `409` the default branch
moved during scaffolding (retry); `422` the GitHub App is not configured/installed
(carries an install hint); `502` GitHub rejected the commit.

```sh
curl -X POST "$FKST_API/api/v1/repos/acme/site/fkst-setup" \
  -H "Authorization: Bearer $TOKEN"

# Re-commit the scaffold over an existing .fkst
curl -X POST "$FKST_API/api/v1/repos/acme/site/fkst-setup?force=true" \
  -H "Authorization: Bearer $TOKEN"
```

---

## Inline secrets & variables

Secrets and non-secret env **variables** for an engine run are supplied **inline
when you trigger a goal** (in the trigger request body) and held by the
controller **in memory only** — there is no persistent vault and no secret CRUD
(the former `/api/v1/vault/entries` endpoints were removed in the database-free
pivot). Each entry has an env-var `key`, a `value`, and a `kind` (`secret` — the
default — or `variable`). They are scoped to the goal's repo, injected into the
engine env at session spawn (the same injection point as before), and **dropped
from memory when the session terminates**.

> **In-memory only.** Inline secrets live in the controller process. They survive
> a **worker** loss (the controller re-dispatches the session and re-injects the
> held values) but **not** a controller restart — a goal re-triggered after a
> restart must re-supply them. Nothing is written to a datastore; values are
> never logged (only key names + a count) and are never returned by any endpoint.

See the [goal trigger](#post-apiv1goalsidtrigger--trigger-a-goal) endpoint's
`secrets` field for the wire shape, and [inline-secret limits](#inline-secret-limits)
for the value-size and per-scope caps.

---

## Appendix: data types & limits

### Enumerations

| Type | Values (wire form) |
|------|--------------------|
| Session status | `pending`, `validating`, `running`, `stopping`, `stopped`, `failed` |
| Goal status | `not_started`, `triggered`, `running`, `stopped`, `failed` |
| GitHub repo mode (goal trigger) | `existing`, `create_new` |
| Inline secret kind | `variable`, `secret` |

### Package layout (repo-scoped)

Packages are **repo-scoped**: they live in the user's GitHub repo under
`<repo>/.fkst/packages/<name>/` and are loaded at session spawn from the cloned
repo (there is no package store or package HTTP API). The engine identifies a
package by its directory **basename**.

| Rule | Value |
|------|-------|
| Package name (directory basename) | `^[A-Za-z0-9_-]+$`, and not the reserved `host` |
| Location | `<repo>/.fkst/packages/<name>/` |
| Engine entry (required) | at least one `departments/<name>/main.lua` or `raisers/<name>.lua` |

### Goal limits

| Limit | Value |
|-------|-------|
| Title | 1–200 characters |
| Description | 1–16384 bytes |
| Packages per goal | 1–16 (each name format-validated; resolved against the repo at spawn) |
| Repo owner | `^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})$` |
| Repo name | `^[A-Za-z0-9._-]{1,100}$` |

### Inline-secret limits

| Limit | Value |
|-------|-------|
| Entry key | `^[A-Za-z_][A-Za-z0-9_]*$` |
| Reserved keys (rejected with `422`) | any `FKST_*`, `GITHUB_TOKEN`, `FKST_GITHUB_TOKEN_FILE`, `FKST_GOAL_FILE`, and the allow-listed host vars (`PATH`, `HOME`, `CODEX_HOME`, `LANG`, `LC_ALL`, `TMPDIR`, `TZ`, `SSL_CERT_FILE`, `SSL_CERT_DIR`) |
| Single value | ≤ 64 KiB (default; `FKST_HOSTED_VAULT_VALUE_BYTE_CAP`) |
| Entries per scope | ≤ 100 (default; `FKST_HOSTED_VAULT_ENTRIES_PER_SCOPE_CAP`) |
| Scope | the goal's repo (`repo:<owner>/<name>`), overlaying the owner-wide `global` scope |
| Storage | in-memory only (controller process); never persisted, returned, or logged (only key names + a count) |
