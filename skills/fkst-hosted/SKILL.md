---
name: fkst-hosted
description: Drive the fkst-hosted HTTP API as an AI agent — authenticate with a NyxID token (direct bearer or the NyxID proxy), then manage fkst packages, run engine sessions, pursue goals against GitHub, and work the GitHub issues hub.
version: 0.1.0
homepage: https://github.com/ChronoAIProject/fkst-hosted
user-invocable: /fkst-hosted
metadata:
  category: plain
  tag:
    - fkst-hosted
    - backend
    - http-api
    - packages
    - sessions
    - goals
    - github
    - nyxid
---

# fkst-hosted

> **You are an AI agent reading this manual to learn how to call the fkst-hosted
> backend.** Throughout, *"you"* means the agent. fkst-hosted is ChronoAI's
> hosted cloud service for the **fkst** project: a managed home for fkst
> **packages** (the lua bundles the engine runs) and the engine that runs them
> as **sessions**, plus **goals** (run a package against a GitHub repo) and a
> **GitHub issues hub**. Everything is a JSON HTTP API under `/api/v1`, secured
> by the user's ChronoAI (NyxID) sign-in.

This manual teaches the **how**: how to authenticate, which transport to use,
and the copy-pasteable workflows for the common tasks. For the exhaustive
per-endpoint contract — every field, permission, and status code — the single
source of truth is **[`docs/api-reference.md`](../../docs/api-reference.md)**
(GitHub-App specifics: [`docs/github-app.md`](../../docs/github-app.md)). A
condensed endpoint cheat-sheet ships alongside this file at
[`references/api-quickref.md`](references/api-quickref.md).

## What you can do

- **Packages** — create / list / fetch / replace / delete a package, upload one
  as a zip, **generate** one from a natural-language description, and **share** it
  with users or orgs.
- **Sessions** — start an engine session for a package, poll its status, stop it.
- **Goals** — capture an intent + packages + a target GitHub repo, edit it, and
  **trigger** it to spawn a session.
- **GitHub issues hub** — list issues across all linked GitHub accounts and
  create / update / comment on them.

---

## 1. Authenticate first

**Every endpoint except the health checks requires a NyxID access token** — an
RS256 JWT — sent as a bearer:

```
Authorization: Bearer <nyxid-access-token>
```

- Missing/malformed/expired token → `401 unauthorized` with a
  `WWW-Authenticate: Bearer` header.
- NyxID (the JWKS issuer) unreachable → `503 unavailable`.
- `GET /health` and `GET /api/v1/health` need **no** auth.

You reach the API over HTTP. **Direct bearer (Transport A) is the recommended,
contract-backed path.** Transport B is an optional NyxID-proxy convenience that
carries the caveat noted there.

### Transport A — direct HTTP (always correct)

Set the deployment base URL and your token, then call with curl. This is the
baseline every example below uses.

```bash
export FKST_API="https://fkst.example.com"     # deployment base, no trailing slash, no /api/v1
export TOKEN="<nyxid-access-token>"             # an RS256 JWT for this deployment's audience

curl -sS "$FKST_API/health"                                   # open, no auth
curl -sS -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/packages"
```

### Transport B — NyxID credential-broker proxy (convenience; see caveat)

You can let NyxID store the bearer and inject the `Authorization` header so your
code never handles the raw token. fkst-hosted is **not in the NyxID catalog
yet**, so register it once as a **custom** service whose `endpoint_url` is the
deployment base (the part before `/api/v1`):

```bash
# One-time: register the deployment as a custom NyxID service. The credential is
# a NyxID access token this fkst deployment accepts as bearer.
nyxid service add --custom            # set slug=fkst-hosted, endpoint_url=$FKST_API, auth=bearer

# Then call it — paths are RELATIVE to endpoint_url, so they keep the /api/v1
# prefix. JSON bodies must arrive as application/json (a non-JSON body is a 400),
# so set the content type explicitly if the CLI does not:
nyxid proxy request fkst-hosted /api/v1/packages -m GET
nyxid proxy request fkst-hosted /api/v1/sessions -m POST \
  -H "Content-Type: application/json" -d '{"package_name":"billing-pipeline"}'
```

The path, method, and body are otherwise identical to Transport A.

> **Caveat — prefer Transport A.** fkst-hosted verifies a short-lived NyxID
> *access JWT* (checked for issuer, audience, and expiry). A token stored as a
> static custom-service credential **will expire and start returning `401`**,
> and nothing in fkst-hosted mints or refreshes a self-audience token for you.
> This transport is an unverified convenience, **not** part of
> [`docs/api-reference.md`](../../docs/api-reference.md); use it only with a
> freshly-supplied, unexpired token. Direct bearer (Transport A) is the
> recommended path. For the NyxID CLI (login, `service add`, approvals, nodes)
> load the **`nyxid`** skill; never ask the user to paste raw tokens into chat.

> **Conventions.** JSON bodies need `Content-Type: application/json`; zip uploads
> need `Content-Type: application/zip`. Unknown JSON fields are rejected with
> `400` (a typo fails loudly). Timestamps are RFC 3339 UTC (`…Z`). Session and
> goal IDs are UUIDs; a malformed UUID in a path is `400`, never `404`. Every
> response carries an `x-request-id` you can quote when reporting a problem.

---

## 2. The endpoint map

All paths are under `/api/v1`. Full detail in
[`docs/api-reference.md`](../../docs/api-reference.md); cheat-sheet in
[`references/api-quickref.md`](references/api-quickref.md).

| Domain | Endpoints |
|--------|-----------|
| Health | `GET /health` · `GET /api/v1/health` (open) |
| Packages | `POST /packages` · `GET /packages` · `GET/PUT/DELETE /packages/{name}` · `POST/PUT /packages/{name}/archive` |
| Generation | `POST /packages/generate` |
| Sharing | `POST/GET /packages/{name}/shares` · `DELETE /packages/{name}/shares/{share_id}` |
| Sessions | `POST /sessions` · `GET /sessions/{id}` · `POST /sessions/{id}/stop` |
| Goals | `POST/GET /goals` · `GET/PATCH/DELETE /goals/{id}` · `POST /goals/{id}/trigger` |
| GitHub hub ⚠ | `GET /github/accounts` · `GET /github/issues` · `GET/POST/PATCH /github/repos/{owner}/{repo}/issues[/{number}]` · `GET/POST .../comments` — depends on a not-yet-shipped NyxID contract; see §3.5 |

---

## 3. Core workflows

### 3.1 Run a package (the happy path)

Create (or generate) a package, start a session, poll it to `running`, then stop
it and poll to `stopped`. Sessions advance asynchronously — **you must poll**.

```bash
# 1) Create a package (must contain an engine entry file:
#    departments/<name>/main.lua  OR  raisers/<name>.lua)
curl -sS -X POST "$FKST_API/api/v1/packages" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"name":"billing-pipeline",
       "files":[{"path":"departments/billing/main.lua","content":"return {}"}],
       "composed_deps":[]}'
# 201 -> {"name":"billing-pipeline"}   (409 if the name already exists)

# 2) Start a session for it
SID=$(curl -sS -X POST "$FKST_API/api/v1/sessions" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"package_name":"billing-pipeline"}' | jq -r .id)
# 201 -> {"id":"<uuid>","status":"pending"}   (409 if a live session already holds it)

# 3) Poll until running (status walks pending -> validating -> running)
curl -sS -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/sessions/$SID" | jq .status

# 4) Stop (async: 202 acknowledges; keep polling until "stopped")
curl -sS -X POST -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/sessions/$SID/stop"
# 202 -> {"status":"stopping"}

# 5) Poll until stopped
curl -sS -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/sessions/$SID" | jq .status
```

Session lifecycle: `pending → validating → running → stopping → stopped`, or
`→ failed` on any error (read the `error` field on a failed session). A session
**materializes the package at start**, so editing the package later does not
affect a running session.

### 3.2 Generate a package from a description

```bash
curl -sS -X POST "$FKST_API/api/v1/packages/generate" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"description":"a department that greets every tick event","save":true}'
```

Returns **`200` even when the draft fails** — read the verdict in the body:
`validation.ok`, `conformance.status` (`ok`/`failed`/`skipped`), `saved`,
`save_error`, `attempts`. With `save:true` the draft is persisted as your own
package only if it **validates AND conformance did not fail**; otherwise it
stays unsaved with a `save_error` (`"validation failed"` or `"conformance
failed"`) — always check `saved`/`save_error`. `503` means generation is not
configured on the deployment **or** the LLM gateway is unreachable.

### 3.3 Share a package

```bash
curl -sS -X POST "$FKST_API/api/v1/packages/billing-pipeline/shares" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"grantee_kind":"user","grantee_id":"user-456","level":"use"}'
```

`level:"read"` grants viewing; `level:"use"` grants viewing **and** starting
sessions. A `read` share does **not** let the grantee run a session. Managing
shares requires Manage permission on the package.

### 3.4 Goals → trigger against GitHub

```bash
# Create a goal (every listed package must be one you can "use")
GID=$(curl -sS -X POST "$FKST_API/api/v1/goals" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"title":"Build a billing pipeline",
       "description":"Create a billing pipeline that processes invoices.",
       "package_names":["billing-pipeline"],
       "repo":{"owner":"acme","name":"billing"}}' | jq -r .id)

# Trigger it (spawns a session against the repo). 202 returns the session_id.
curl -sS -X POST "$FKST_API/api/v1/goals/$GID/trigger" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"repo":{"owner":"acme","name":"billing"}}'
# 202 -> {"goal_id":"…","session_id":"…","goal_status":"triggered","session_status":"pending"}
```

Then poll the returned `session_id` exactly as in 3.1. Triggering needs the
**fkst-hosted GitHub App installed** on the target repo (else `422`). Use
`{"repo_mode":"create_new","create":{...}}` to have a repo created first —
`create_new` additionally requires NyxID's credential proxy configured (else
`503`) and depends on the same not-yet-delivered NyxID GitHub contract noted in
3.5, so prefer `existing` mode for now. Packages/repo on a goal can be changed
only while its status is `not_started`, `stopped`, or `failed`;
title/description are always editable. Goal lifecycle:
`not_started → triggered → running → stopped`, or `→ failed`.

### 3.5 GitHub issues hub

```bash
curl -sS -H "Authorization: Bearer $TOKEN" "$FKST_API/api/v1/github/accounts"
curl -sS -H "Authorization: Bearer $TOKEN" \
  "$FKST_API/api/v1/github/issues?filter=assigned&state=open&per_page=50"
curl -sS -X POST "$FKST_API/api/v1/github/repos/acme/billing/issues" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"title":"Invoices double-counted","body":"Steps…","labels":["bug"]}'
```

GitHub is reached only through NyxID's credential proxy using **the user's**
OAuth grant — fkst-hosted never holds a GitHub token. Once the proxy and token
exchange succeed, `GET /github/issues` aggregates across accounts and is `200`:
a failing account is reported in its own per-result `error` block, never failing
the whole request. A rejected NyxID token exchange returns `401` and a
missing/unreachable credential proxy returns `503` **before** any aggregation,
so "always 200" holds only after the proxy resolves. When the user has
**several** linked accounts, single-target calls require an `account`
field/param (a linked login); with one account it is implied — an absent/unknown
`account` on a single-target call is `422`.

> **⚠ Not yet functional end-to-end.** The hub depends on a NyxID
> connections-listing + GitHub-proxy contract (the per-account `{connection_id,
> login, primary}` projection and the GitHub proxy route) that the fkst-hosted
> source itself marks **UNVERIFIED / not yet shipped in NyxID main**. Against a
> real deployment today these calls may return `503` or empty until that NyxID
> contract lands — treat the hub as forthcoming, not guaranteed-working, and
> verify with `GET /github/accounts` before relying on it.

---

## 4. Errors & status codes

Every error is one JSON envelope: `{"error":"<code>","message":"<text>"}`.
`error` is a stable machine code; `5xx` messages are always the fixed
`"internal server error"` (details are logged, never returned).

| Status | Code | When | Notes |
|--------|------|------|-------|
| 400 | `invalid_request` | Bad body, invalid name/field, unknown JSON field, malformed UUID | |
| 401 | `unauthorized` | Missing/invalid token | `WWW-Authenticate: Bearer` |
| 403 | `forbidden` | Authenticated but not permitted (denied Write/Manage) | |
| 404 | `not_found` | Absent **or hidden by anti-enumeration** | A resource you can't read reads as absent |
| 409 | `conflict` | Duplicate name, busy package/lease, illegal transition | |
| 422 | `unprocessable` | GitHub App not installed, missing dependent resource, account selection | |
| 429 | `rate_limited` | GitHub upstream rate-limited | `Retry-After: <seconds>` |
| 500 | `internal` | Unexpected server error | message is fixed |
| 502 | `upstream_error` | GitHub (via proxy) returned an unexpected error | |
| 503 | `unavailable` | DB / NyxID / LLM gateway / credential proxy down | |
| 408 | — | Request exceeded the server timeout | retry |

**Authorization model (summary).** Resources carry an optional `owner_user_id`
and `org_id`. Access is granted by, in order: the `fkst:admin` scope → the owner
→ the caller's org role (Viewer=read, Member=+write, Admin=+manage) →
legacy-open (no owner). Packages also honor **shares** (`read` / `use`). A
denied **Read** on a private resource returns `404` (anti-enumeration); a denied
**Write/Manage** returns `403`.

---

## 5. Working rules

- **Authenticate every `/api/v1` call.** Only the health checks are open.
- **Never log or echo the bearer token, GitHub tokens, or issue/description
  bodies.** Prefer Transport B so NyxID handles the credential.
- **Poll, don't assume.** `POST /sessions` and `…/stop` and `…/goals/{id}/trigger`
  are asynchronous — `201`/`202` only acknowledge; the real state comes from
  polling `GET /sessions/{id}`.
- **A `404` may mean "not yours."** Anti-enumeration hides resources you can't
  read; don't treat it as proof of non-existence for a name you chose.
- **One live session per package.** A second start while one is live is `409`;
  stop the first (and poll to `stopped`) before reusing the package.
- **Editing a package doesn't touch running sessions** — files are snapshotted
  at start. Restart to pick up changes.
- **`generate` returns `200` even on failure** — always read `validation`/
  `conformance` before assuming success.
- **Respect `Retry-After`** on `429`; pace GitHub writes using the
  `x-ratelimit-*` headers the create/update responses echo.
- **Don't guess endpoints or fields.** When unsure, consult
  [`docs/api-reference.md`](../../docs/api-reference.md) — never invent a path.

## 6. Limits (quick)

Package name `^[A-Za-z0-9_-]+$`; ≤256 files; path ≤512 bytes (forward-slash
only, no `..`/absolute/backslash/control chars, UTF-8); file content ≤1 MiB;
total content ≤12 MiB; `composed_deps` ≤256 entries, each ≤256 bytes (no
newline/NUL). Goal title 1–200 bytes; description 1–16384 bytes; 1–16 packages.
Generate description 1–8192 bytes. Full tables:
[`references/api-quickref.md`](references/api-quickref.md).
