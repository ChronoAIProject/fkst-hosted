# fkst-hosted API quick reference

A dense, one-line-per-endpoint cheat-sheet for agents, so this skill is
self-contained when separated from the repo. The **authoritative**, fuller catalogue —
every field, permission nuance, and example — is
[`docs/api-reference.md`](../../../docs/api-reference.md) in the fkst-hosted repo.
The status codes and limits below are verified against the backend source, so on
the rare point where they are more specific than that catalogue, they reflect
the code. See [`SKILL.md`](../SKILL.md) for authentication and worked workflows.

## Auth & conventions

- All `/api/v1/*` routes require `Authorization: Bearer <nyxid-access-token>`
  (RS256 JWT). `GET /health` and `GET /api/v1/health` are open.
- JSON bodies: `Content-Type: application/json`. Zip uploads:
  `Content-Type: application/zip`. Unknown JSON fields → `400`.
- Timestamps are RFC 3339 UTC (`…Z`). Session/goal IDs are UUIDs; a malformed
  path UUID is `400`. Every response carries `x-request-id`.
- `$FKST_API` = deployment base (e.g. `https://fkst.example.com`), no `/api/v1`,
  no trailing slash.

## Authorization model

Order of decision: `fkst:admin` scope → owner (`owner_user_id`) → org role
(Viewer=Read, Member=+Write, Admin=+Manage, resolved via NyxID) → legacy-open
(no owner). Packages also honor **shares**: `read` = Read; `use` = Read + start
a session. Denied **Read** on a private resource → `404` (anti-enumeration);
denied **Write/Manage** → `403`. Actions: Read=fetch, Write=update, Manage=delete
/ manage shares.

## Error envelope

`{"error":"<code>","message":"<text>"}`. Codes: `invalid_request` (400),
`unauthorized` (401, `WWW-Authenticate: Bearer`), `forbidden` (403),
`not_found` (404), `conflict` (409), `unprocessable` (422), `rate_limited`
(429, `Retry-After`), `internal` (500, fixed message), `upstream_error` (502),
`unavailable` (503). `408` (timeout) has no envelope.

---

## Health

| Method & path | Auth | Purpose | Codes |
|---------------|------|---------|-------|
| `GET /health`, `GET /api/v1/health` | none | Liveness + DB ping | `200 {status,mongo,version}` · `503` degraded |

## Packages

| Method & path | Permission | Purpose | Key codes |
|---------------|------------|---------|-----------|
| `POST /api/v1/packages` | auth (org Member/Admin if `org_id`) | Create from JSON | `201 {name}`+`Location` · `400` · `409` dup |
| `GET /api/v1/packages` | auth | List names you can see (`?filter=shared` = only shared) | `200 ["…"]` |
| `GET /api/v1/packages/{name}` | Read | Fetch one package | `200 Package` · `400` · `404` |
| `PUT /api/v1/packages/{name}` | Write | Replace `files`+`composed_deps` (snapshot-at-start) | `200 Package` · `400` · `403` · `404` |
| `DELETE /api/v1/packages/{name}` | Manage | Delete (cascades shares) | `204` · `403` · `404` · `409` busy |
| `POST /api/v1/packages/{name}/archive` | auth | Create from raw zip (`application/zip`) | `201 {name}` · `400` · `409` |
| `PUT /api/v1/packages/{name}/archive` | Write | Replace from raw zip | `200 Package` · `400` · `403` · `404` |

**Request body — create:** `name` (req, `^[A-Za-z0-9_-]+$`), `files` (req, ≥1,
must include an engine entry), `composed_deps` (opt, default `[]`), `org_id`
(opt). **PUT:** `files` (req), `composed_deps` (opt). **PackageFile:**
`{path, content}`. **Package response:** `{name, files, composed_deps,
owner_user_id, org_id, created_at, updated_at}`.

**Zip rules:** ≤256 entries + optional root `composed.deps` (parsed into
`composed_deps`, not stored); per-file ≤1 MiB, total ≤12 MiB, UTF-8; root
`fkst.env` rejected; same path rules as JSON create.

## Package generation

| Method & path | Permission | Purpose | Key codes |
|---------------|------------|---------|-----------|
| `POST /api/v1/packages/generate` | auth (`save:true` → your package) | Generate a draft via NyxID LLM gateway | `200` (always when it runs) · `400` · `409` · `503` |

**Body:** `description` (req, 1–8192 bytes), `name` (opt, `^[A-Za-z0-9_-]+$`,
else minted `gen-<hex>`), `save` (opt bool). **Response (`200`):**
`{package, validation:{ok,errors}, conformance:{status,errors,skipped_reason},
saved, save_error, attempts}`. `conformance.status` ∈ `ok|failed|skipped`.
`200` is returned **even when the draft fails** — read the verdict.

## Package sharing

| Method & path | Permission | Purpose | Key codes |
|---------------|------------|---------|-----------|
| `POST /api/v1/packages/{name}/shares` | Manage | Grant a share | `201 ShareView` · `400` · `403` · `404` · `409` · `503` |
| `GET /api/v1/packages/{name}/shares` | Manage | List shares | `200 [ShareView]` |
| `DELETE /api/v1/packages/{name}/shares/{share_id}` | Manage | Revoke a share | `204` · `400` · `403` · `404` |

**Body — create share:** `grantee_kind` (`user`|`org`), `grantee_id`, `level`
(`read`|`use`). **ShareView:** `{id, package_name, grantee_kind, grantee_id,
level, granted_by, created_at}`. Create returns `404` if the package is absent
and `503` if a NyxID user/org lookup is unavailable.

## Sessions

| Method & path | Permission | Purpose | Key codes |
|---------------|------------|---------|-----------|
| `POST /api/v1/sessions` | `use` on the package | Start a session | `201 {id,status:"pending"}`+`Location` · `400` · `403` · `404` · `409` lease |
| `GET /api/v1/sessions/{id}` | Read | Fetch status | `200 SessionView` · `400` · `404` |
| `POST /api/v1/sessions/{id}/stop` | Write | Request a stop (async) | `202 {status:"stopping"}` · `400` · `403` · `404` |

**Body — start:** `package_name` (req, `^[A-Za-z0-9_-]+$`, ≤128 bytes).
**SessionView:** `{id, package_name, package_names[], status, error,
owner_user_id, org_id, goal_id, repo, triggered_by, created_at, started_at,
stopped_at, + runtime: pod_id, fencing_token, pid, runtime_dir}`. Status ∈
`pending|validating|running|stopping|stopped|failed`. `stop` is idempotent (`202`
for a real transition or a no-op); poll `GET` until `stopped`. One live session
per package (second start → `409`). Files are snapshotted at start.

## Goals

| Method & path | Permission | Purpose | Key codes |
|---------------|------------|---------|-----------|
| `POST /api/v1/goals` | auth (org M/A if `org_id`; pkgs must be usable) | Create a goal | `201 GoalView`+`Location` · `400` · `403` · `503` |
| `GET /api/v1/goals` | auth | List own + org goals (`?status`, `?limit`≤200, `?offset`) | `200 [GoalView]` |
| `GET /api/v1/goals/{id}` | Read | Fetch a goal | `200 GoalView` · `400` · `404` |
| `PATCH /api/v1/goals/{id}` | Write | Partial update | `200 GoalView` · `400` · `403` · `404` · `409` |
| `DELETE /api/v1/goals/{id}` | Manage | Delete | `204` · `403` · `404` · `409` |
| `POST /api/v1/goals/{id}/trigger` | owner / org Member-Admin | Spawn a session vs a repo | `202` · `400` · `403` · `404` · `409` · `422` · `503` |

An unknown package on create is `400` (validation), not `404`. Trigger in
`create_new` mode additionally needs the NyxID proxy and can surface
`401`/`500`/`503` from repo creation (and the not-yet-shipped NyxID GitHub
contract — see the hub note below).

**Body — create:** `title` (1–200 bytes), `description` (1–16384 bytes),
`package_names` (1–16 usable, no dups), `repo` (`{owner,name}`, opt), `org_id`
(opt). **PATCH (all opt):** `title`, `description`, `package_names`, `repo`,
`clear_repo` (mutually exclusive with `repo`); `package_names`/`repo` only while
status is mutable (`not_started`/`stopped`/`failed`). **Trigger body:**
`repo_mode` (`existing`(default)|`create_new`), `repo` (existing only),
`create` (`{name, private=true, description?, org_login?}`, required for
`create_new`). **Trigger `202`:** `{goal_id, session_id, goal_status,
session_status}`. **GoalView:** `{id, title, description, package_names, repo,
status, owner_user_id, org_id, active_session_id, created_at, updated_at}`.
Status ∈ `not_started|triggered|running|stopped|failed`. Trigger needs the
fkst-hosted GitHub App installed on the repo (else `422`).

## GitHub issues hub

GitHub is reached only via NyxID's credential proxy (RFC 8693 delegation) using
the user's OAuth grant; fkst-hosted never holds a GitHub token. `account` (a
linked login) is implied with one linked account, **required** with several
(absent/unknown → `422` on single-target calls). **⚠ The underlying NyxID
connections-listing + GitHub-proxy contract is UNVERIFIED in NyxID main, so hub
calls may return `503`/empty until it ships.** A rejected NyxID token exchange
returns `401` before any per-account work.

| Method & path | Purpose | Key codes |
|---------------|---------|-----------|
| `GET /api/v1/github/accounts` | List linked accounts | `200 [AccountView]` · `401` · `503` |
| `GET /api/v1/github/issues` | Aggregate issues across accounts | `200 {results[]}` (after proxy resolves) · `401` · `503` |
| `POST /api/v1/github/repos/{owner}/{repo}/issues` | Create an issue | `201 IssueView` · `400` · `422` · `429` · `502` |
| `GET /api/v1/github/repos/{owner}/{repo}/issues/{number}` | Fetch one issue (with body) | `200` · `404` · `422` · `429` · `502` |
| `PATCH /api/v1/github/repos/{owner}/{repo}/issues/{number}` | Update an issue | `200` · `404` · `422` · `429` · `502` |
| `GET …/issues/{number}/comments` | List comments | `200 [CommentView]` · `404` · `422` · `429` · `502` |
| `POST …/issues/{number}/comments` | Add a comment | `201 CommentView` · `400` · `422` · `429` · `502` |

**`GET /github/issues` params:** `accounts` (csv logins), `filter` (default
`assigned`), `state` (`open`(default)/`closed`/`all`), `labels` (csv), `page`
(1), `per_page` (default 30, clamped 1..=50). Once the proxy resolves it is
`200`; per-account failures appear in each result's `error` block (`kind` ∈
`rate_limited|auth|upstream|network`). A rejected token exchange bubbles up as
`401` and a delegation/account-listing failure as `503`. **Comments-list
params:** `account` (cond), `page` (1), `per_page` (30, clamped 1..=50).
**Create issue body:** `title` (req, non-empty), `body`, `labels`, `assignees`,
`account` (cond). **PATCH body (all opt):** `title`, `body`, `state`, `labels`,
`assignees`, `account`. **Comment body:** `body` (req, non-empty), `account`
(cond). Upstream mapping: GitHub `404`→`404`; `401`/`403` (no rate-limit)→`403`;
`422`→`422`; rate-limited→`429`+`Retry-After`; any other (unhandled) status→`502`.

**AccountView:** `{connection_id, login, primary}`. **IssueView:** `{account,
repository, number, id, title, body, state, labels[], assignees[], comments,
html_url, created_at, updated_at}` — `body` is populated on every single-target
response (GET one issue, POST create, PATCH update) and suppressed (`null`) only
in the aggregated `GET /github/issues` list. **CommentView:** `{id, user, body,
html_url, created_at, updated_at}`.

---

## Enumerations

| Type | Values |
|------|--------|
| Session status | `pending`, `validating`, `running`, `stopping`, `stopped`, `failed` |
| Goal status | `not_started`, `triggered`, `running`, `stopped`, `failed` |
| Share grantee kind / level | `user`/`org` · `read`/`use` |
| Conformance status | `ok`, `failed`, `skipped` |
| Goal trigger repo mode | `existing`, `create_new` |

## Limits

| Limit | Value |
|-------|-------|
| Package name | `^[A-Za-z0-9_-]+$` |
| Files per package | 256 |
| File path length | 512 bytes (forward-slash only; no `..`/absolute/backslash/control; UTF-8) |
| File content / total content | 1 MiB / 12 MiB |
| `composed_deps` | 256 entries, each ≤256 bytes (no newline/NUL) |
| Engine entry (required) | ≥1 `departments/<name>/main.lua` or `raisers/<name>.lua` |
| Goal title / description | 1–200 bytes / 1–16384 bytes |
| Packages per goal | 1–16 |
| Repo owner / name (goals only) | `^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})$` / `^[A-Za-z0-9._-]{1,100}$` |
| Generate description | 1–8192 bytes |

> The repo owner/name regexes apply to **goals** (repo create/target). The
> GitHub-issues-hub `{owner}/{repo}` path params are only required to be
> non-empty (else `400`); they are not regex-validated.
