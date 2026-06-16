# fkst-hosted backend

The **fkst-hosted** Rust backend is a three-crate Cargo workspace under
`backend/`. It stores fkst Lua packages in MongoDB and runs
[fkst-substrate](https://github.com/ChronoAIProject/fkst-substrate) sessions on
behalf of users. This is the v1 scope; this README covers **local development**
only.

## Prerequisites

- **Rust** (stable toolchain — pinned by `rust-toolchain.toml`, which also pulls in `rustfmt` and `clippy`)
- **Docker** (for the local MongoDB and the integration tests)

## Local dev quickstart

1. Start MongoDB 7 (data persists in the named volume `fkst_mongo_data`):

   ```sh
   docker compose -f backend/docker-compose.yml up -d
   ```

2. Run the control-plane (from `backend/`). `MONGODB_URI` is required — the
   server fails closed at startup if it is missing or Mongo is unreachable:

   ```sh
   MONGODB_URI="mongodb://localhost:27017" cargo run -p fkst-control-plane
   ```

3. Check health:

   ```sh
   curl -s localhost:8080/health
   curl -s localhost:8080/api/v1/health
   ```

   With Mongo up, both return `200 OK`:

   ```json
   {"status":"ok","mongo":"up","version":"0.0.0"}
   ```

   (`version` is the crate's `CARGO_PKG_VERSION`.)

4. To see the degraded path, stop Mongo while the API is running
   (`docker compose -f backend/docker-compose.yml stop`); both endpoints
   then return `503 Service Unavailable`:

   ```json
   {"status":"degraded","mongo":"down","version":"0.0.0"}
   ```

   Restart with `docker compose -f backend/docker-compose.yml start`.

### Host port 27017 already in use

`backend/docker-compose.yml` maps `27017:27017`. If another Mongo already
occupies host port 27017, change the **host** side of the mapping (e.g.
`"27018:27017"`) and point the API at it:

```sh
MONGODB_URI="mongodb://localhost:27018" cargo run -p fkst-control-plane
```

### Local development & auth

For details on setting up authentication and GitHub App credentials during local development vs production (using Profile templates and the environment variable matrix), see the **[Authentication & GitHub Integration Guide](../docs/auth-integration.md)**.

## Configuration (environment variables)

All configuration is read from the environment at startup (`src/config.rs` is
authoritative). Invalid values (non-numeric ports/timeouts, a zero request
timeout) are rejected at startup with a clear error.

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `MONGODB_URI` | **yes** | — | MongoDB connection string. Missing → the process logs a config error and exits non-zero (fail-closed). Credentials embedded in the URI are **never logged** — the userinfo segment is redacted (`mongodb://<redacted>@host:27017`). |
| `MONGODB_DB` | no | `fkst_hosted` | Logical MongoDB database name. |
| `MONGODB_SERVER_SELECTION_TIMEOUT_MS` | no | `5000` | Driver server-selection timeout (ms). Bounds the startup ping and every `/health` check, so an unreachable Mongo fails fast instead of hanging. |
| `FKST_HOSTED_PORT` | no | `8080` | TCP port the HTTP server binds. |
| `FKST_HOSTED_BIND_ADDR` | no | `0.0.0.0` | Bind address. |
| `FKST_HOSTED_LOG_LEVEL` | no | `info` | `tracing-subscriber` `EnvFilter` directive (e.g. `debug`, `fkst_control_plane=debug`). An invalid directive falls back to `info` with a warning. Logs are JSON. |
| `FKST_HOSTED_REQUEST_TIMEOUT_SECS` | no | `30` | Per-request timeout in seconds (`408 Request Timeout` on expiry). Must be ≥ 1; `0` is rejected at startup. |
| `FKST_AUTH_ENABLED` | no | `true` | Enable NyxID JWT authentication. Set to `"false"` for local dev (all routes open, extractor yields dev context). Default is fail-closed: auth is on unless explicitly disabled. |
| `FKST_AUTH_NYXID_BASE_URL` | when auth enabled | — | NyxID base URL for the JWKS endpoint (e.g. `https://nyxid.example.com`). Trailing `/` is trimmed. Required when `FKST_AUTH_ENABLED=true`. |
| `FKST_AUTH_ISSUER` | no | `nyxid` | Expected JWT `iss` claim. |
| `FKST_AUTH_AUDIENCE` | no | same as base URL | Expected JWT `aud` claim. Defaults to the (trimmed) `FKST_AUTH_NYXID_BASE_URL`. |
| `FKST_AUTH_JWKS_CACHE_TTL_SECS` | no | `300` | JWKS cache TTL in seconds. Must be ≥ 1; `0` is rejected at startup. After TTL expiry, stale keys are served if the refresh fetch fails. |
| `NYXID_CLIENT_ID` | no | — | NyxID service-account client ID for org APIs (e.g. `sa_…`). Both-or-neither with `NYXID_CLIENT_SECRET`. Without both, org features degrade gracefully (owner-only authorization). |
| `NYXID_CLIENT_SECRET` | no | — | NyxID service-account client secret (SECRET). Both-or-neither with `NYXID_CLIENT_ID`. |
| `FKST_NYXID_ORG_CACHE_TTL_SECS` | no | `30` | TTL in seconds for NyxID org-role and user-orgs caches. Controls how stale org membership data may be. Must be ≥ 1; `0` is rejected at startup. |
| `FKST_HOSTED_LLM_GATEWAY_URL` | no | — | NyxID LLM-gateway base URL (NyxID's `{base}/api/v1/llm/gateway/v1`) for `POST /api/v1/packages/generate`. Absent → generation is disabled (the endpoint answers `503`). When set, it **requires** `NYXID_CLIENT_ID`/`NYXID_CLIENT_SECRET` (the service account that mints the `llm:proxy` bearer) and `FKST_HOSTED_LLM_MODEL` — both are rejected at startup if missing. Non-secret (logged). |
| `FKST_HOSTED_LLM_MODEL` | when gateway set | — | Model name the gateway routes by (e.g. `claude-sonnet`). Required when `FKST_HOSTED_LLM_GATEWAY_URL` is set; fail-closed. |
| `FKST_HOSTED_LLM_TIMEOUT_SECS` | no | `20` | Per-request timeout (seconds) for one LLM completion call. Must be ≥ 1; `0` is rejected at startup. |
| `FKST_HOSTED_LLM_MAX_OUTPUT_BYTES` | no | `1048576` | Max bytes accepted from a single completion before the draft is rejected and a corrective retry is attempted. Must be ≥ 1; `0` is rejected at startup. |

> **Deployment note (generation enabled):** the conformance dry-run runs inside
> the HTTP request, so set `FKST_HOSTED_REQUEST_TIMEOUT_SECS` to **≥ 90** when
> `FKST_HOSTED_LLM_GATEWAY_URL` is set — the request budget must cover up to two
> LLM round-trips plus the (≤ 20 s) engine conformance pre-flight.

### Claiming legacy packages and sessions

Pre-auth packages and sessions have no `owner_user_id` field and are grandfathered open to any authenticated user. To assign ownership of legacy docs, run a one-off `mongosh` snippet:

```js
// Assign all ownerless packages to a specific user:
db.packages.updateMany(
  { owner_user_id: { $exists: false } },
  { $set: { owner_user_id: "<user-id>" } }
);
// Assign all ownerless sessions to a specific user:
db.sessions.updateMany(
  { owner_user_id: { $exists: false } },
  { $set: { owner_user_id: "<user-id>" } }
);
```

## Health endpoints

`GET /health` and `GET /api/v1/health` share the same handler: each request
performs a real Mongo `ping`.

| Mongo | Status | Body |
|-------|--------|------|
| reachable | `200 OK` | `{"status":"ok","mongo":"up","version":"<crate version>"}` |
| unreachable | `503 Service Unavailable` | `{"status":"degraded","mongo":"down","version":"<crate version>"}` |

The ping is bounded by the driver's server-selection timeout
(`MONGODB_SERVER_SELECTION_TIMEOUT_MS`, default 5000 ms), so a dead Mongo
yields a fast 503 instead of a hang. The underlying ping error is logged,
never echoed to the client.

> **Kubernetes probe coupling:** a probe's `timeoutSeconds` must **exceed**
> `MONGODB_SERVER_SELECTION_TIMEOUT_MS` (default 5 s — so use e.g.
> `timeoutSeconds: 6` or higher). Otherwise the probe times out before the
> handler can answer with the diagnostic 503 body.

## Package API endpoints

All package endpoints require authentication (bearer token). Session
materialization uses **snapshot semantics**: sessions materialize package
files at spawn — a PUT affects only sessions started afterwards.

| Method | Path | Status | Description |
|--------|------|--------|-------------|
| `POST` | `/api/v1/packages` | 201 | Create package (JSON body with `name`, `files`, optional `composed_deps`, `org_id`) |
| `GET` | `/api/v1/packages` | 200 | List visible package names (ascending) |
| `GET` | `/api/v1/packages/{name}` | 200 | Fetch one package |
| `PUT` | `/api/v1/packages/{name}` | 200 | Replace `files` and `composed_deps` (JSON body; `created_at` and ownership untouched) |
| `DELETE` | `/api/v1/packages/{name}` | 204 | Delete package (409 if active session or live lease exists) |
| `POST` | `/api/v1/packages/{name}/archive` | 201 | Create package from zip archive (`Content-Type: application/zip`) |
| `PUT` | `/api/v1/packages/{name}/archive` | 200 | Replace package from zip archive (`Content-Type: application/zip`) |
| `POST` | `/api/v1/packages/generate` | 200 | Generate a package draft from a natural-language `description` (LLM); see below |

### LLM package generation (`POST /api/v1/packages/generate`)

Generate a validated fkst package draft from a natural-language `description`.
Requires the LLM gateway to be configured (`FKST_HOSTED_LLM_GATEWAY_URL`); when
it is not, the endpoint answers `503`.

**Request** (JSON):

```json
{ "description": "a department that greets every tick event",
  "name": "my-pkg",      // optional; a unique gen-<hex> name is minted when absent
  "save": false }         // optional; persist the draft as your own package when it validates
```

- `description` — 1..=8192 bytes (a `400` otherwise).
- `name` — optional; when present it must match `^[A-Za-z0-9_-]+$` (a `400`
  otherwise). When absent a unique `gen-<8 hex>` name is generated.
- `save` — when `true`, a **validated** draft whose conformance did not fail is
  persisted as the caller's own package; otherwise `save_error` records why and
  nothing is stored.

**Response** (`200 OK` — even when the draft fails validation/conformance):

```json
{
  "package": { "name": "my-pkg", "files": [ { "path": "...", "content": "..." } ], "composed_deps": [] },
  "validation": { "ok": true, "errors": [] },
  "conformance": { "status": "ok", "errors": [], "skipped_reason": null },
  "saved": false,
  "save_error": null,
  "attempts": 1
}
```

- `validation.ok` — the SAME `NewPackage::validate` gate every uploaded package
  passes. A draft that fails it is reported with `ok:false` and `errors`, and
  one corrective retry (with the validation errors fed back to the model) is
  attempted before giving up.
- `conformance.status` — `ok` / `failed` / `skipped`. The optional engine
  conformance dry-run runs only when the draft validates and the request budget
  allows; a raiser-only draft, a missing engine binary, or an exhausted budget
  yields `skipped` (with `skipped_reason`).
- `attempts` — `1` or `2`.

**Status codes:** `200` (generation ran), `400` (empty/oversize description or
invalid explicit name), `409` (`save:true` collided with an existing name),
`503` (generation not configured, or the gateway is unreachable).

**Trust model & privacy.** The model is reached through NyxID's LLM gateway
using a service-account bearer (scope `llm:proxy`); fkst-hosted never sees a raw
provider key. The LLM has **no tool access** and never touches the host — the
generated package is schema-parsed and then hard-validated by the exact gate
every uploaded package passes, so a **generated package is exactly as trusted as
a user-uploaded one** (it runs under the engine like everything else). The
caller's `description`, the prompts, and the raw model output are **never
logged** — only byte sizes, file counts, the attempt count, and the conformance
status.

### Zip archive upload

Upload a zip file as raw `application/zip` bytes (not multipart):

```sh
curl --data-binary @pkg.zip \
  -H "Content-Type: application/zip" \
  -H "Authorization: Bearer $TOKEN" \
  http://localhost:8080/api/v1/packages/my-pkg/archive
```

Constraints enforced during zip extraction:
- Max 256 file entries (plus one optional root `composed.deps`)
- Per-file content: max 1 MiB
- Total decoded content: max 12 MiB (zip-bomb guard)
- All content must be valid UTF-8
- Root `fkst.env` is rejected (host-owned file)
- Root `composed.deps` is parsed into `composed_deps` (not stored as a file)
- Path rules enforced by `NewPackage::validate` (no `..`, `/`, `\`, control chars)

### Authorization

- **Read**: owner, org viewer+, admin scope
- **Write** (PUT, PUT-archive): owner, org member+, admin scope
- **Manage** (DELETE): owner, org admin, admin scope
- Foreign private packages return 404 (anti-enumeration)

## GitHub issues hub API

Aggregate a user's GitHub issues across **all** their linked GitHub accounts and
run single-target issue operations (create / read / update / comment). GitHub is
reached **only** through NyxID's credential-injection proxy with an RFC 8693
delegated token — fkst-hosted never holds a GitHub token. All endpoints require
authentication. Issue bodies and tokens are never logged (only counts/sizes).

| Method | Path | Status | Description |
|--------|------|--------|-------------|
| `GET` | `/api/v1/github/accounts` | 200 | List the caller's linked GitHub accounts (`connection_id`, `login`, `primary`) |
| `GET` | `/api/v1/github/issues` | 200 | Aggregate issues across linked accounts (resilient fan-out; see below) |
| `POST` | `/api/v1/github/repos/{owner}/{repo}/issues` | 201 | Create an issue (`title`, optional `body`/`labels`/`assignees`/`account`) |
| `GET` | `/api/v1/github/repos/{owner}/{repo}/issues/{number}` | 200 | Fetch one issue (body populated) |
| `PATCH` | `/api/v1/github/repos/{owner}/{repo}/issues/{number}` | 200 | Update an issue (`title`/`body`/`state`/`labels`/`assignees`/`account`) |
| `GET` | `/api/v1/github/repos/{owner}/{repo}/issues/{number}/comments` | 200 | List an issue's comments |
| `POST` | `/api/v1/github/repos/{owner}/{repo}/issues/{number}/comments` | 201 | Add a comment (`body`, optional `account`) |

### Aggregate (`GET /api/v1/github/issues`)

Query params: `accounts` (comma-separated logins to restrict to; default all),
`filter` (default `assigned`), `state` (default `open`), `labels`
(comma-separated; each URL-encoded individually upstream), `page` (default `1`),
`per_page` (default `30`, clamped to `1..=50`).

The response is **always `200`** once the account listing resolves — a slow or
failing account never sinks the whole request. Each account is queried
concurrently (10 s budget each) and reported separately:

```json
{ "results": [
  { "account": "octocat", "issues": [ /* IssueView, body suppressed in lists */ ],
    "page": 1, "per_page": 30, "has_more": true,
    "rate_limit": { "remaining": 4998, "reset_epoch": 1700000000 } },
  { "account": "hubber", "issues": [], "page": 1, "per_page": 30, "has_more": false,
    "error": { "kind": "rate_limited", "message": "github rate limited; retry later",
               "retry_after_secs": 41 } }
] }
```

Per-account `error.kind` is one of `rate_limited` | `auth` | `upstream` |
`network`. `has_more` is derived from GitHub's `Link: …; rel="next"` header.
Only a delegation / connection-listing failure bubbles up as a `503`. Zero
linked accounts yields `{ "results": [] }` (still `200`).

### Account selection (single-target ops)

`account` (a linked GitHub login) selects which linked account to act under. It
is **implied** when exactly one account is linked; when several are linked it is
**required** — an absent `account` yields `422 "multiple GitHub accounts linked;
specify account"`, and an unknown login yields `422`.

### Upstream status mapping (single-target ops)

GitHub `404` → `404`; `401` / `403`-without-rate-limit → `403`; `422` → `422`
(surfacing GitHub's first error message); `403`/`429` with rate-limit evidence →
`429` with a `Retry-After` header; any other `5xx` → `502 upstream_error`.
Successful create/read responses copy GitHub's `x-ratelimit-*` headers through.

## Testing

```sh
cargo test --workspace
```

- Unit tests and the router tests (`tests/health.rs`) run anywhere — no
  Docker, no real TCP bind.
- The Mongo integration tests (`tests/persistence.rs`) start an ephemeral
  MongoDB container via `testcontainers` and **skip cleanly when Docker is
  absent**, so the suite stays green on Docker-less runners.

## Workspace layout (three crates)

The backend is a Cargo workspace with three crates:

| Crate | Kind | Role |
|-------|------|------|
| `fkst-shared` | lib | Role-neutral models and transport clients shared by both binaries. Holds domain models, the NyxID client, LLM client, Ornn types, and vault models. Depends on `bson` — **not** on `mongodb` or `axum`. |
| `fkst-control-plane` | bin | The controller — today's full application (renamed from `fkst-hosted-api`). Owns the public REST API, MongoDB, goals, vault service, and GitHub App integration. Tracing target: `fkst_control_plane`. Binary: `fkst-control-plane`. |
| `fkst-worker` | bin | The worker deployable — compiling skeleton for now (logs its role, waits for SIGTERM). Engine driver, registry client, and pull loop arrive in later issues (#134, #136, #140). Tracing target: `fkst_worker`. |

```
backend/
├── Cargo.toml                  # cargo workspace (members: fkst-shared, fkst-control-plane, fkst-worker)
├── docker-compose.yml          # local dev MongoDB 7 (named volume fkst_mongo_data)
├── engine.ref                  # pinned fkst-substrate commit SHA
├── rust-toolchain.toml         # stable + rustfmt + clippy
├── fkst-shared/
│   └── src/                    # models, nyxid client, llm client, ornn::types, vault::model
├── fkst-control-plane/
│   ├── Dockerfile             # slim controller image (engine-free; see §Docker images)
│   ├── src/
│   │   ├── main.rs             # entrypoint: JSON tracing, config, Mongo connect, graceful shutdown
│   │   ├── lib.rs              # module exports (binary + integration tests share them)
│   │   ├── auth/               # NyxID JWT authentication: JWKS cache, verification, middleware
│   │   ├── authz.rs            # Resource authorization: owner/org role policy
│   │   ├── config.rs           # typed Config from env (FKST_HOSTED_* + MONGODB_*)
│   │   ├── db.rs               # Db handle: typed collections, ping, idempotent indexes, URI redaction
│   │   ├── distribution/       # Session distribution, health view, reaper
│   │   ├── engine/             # Engine runner (process management)
│   │   ├── error.rs            # AppError -> canonical JSON error envelope
│   │   ├── github_app/         # GitHub App integration (tokens, repo access)
│   │   ├── journal/            # Session journaling to GitHub
│   │   ├── leases/             # Per-package lease store (mutual exclusion)
│   │   ├── nyxid/              # NyxID client (org-role lookups) — types live in fkst-shared
│   │   ├── packages/           # Package domain: validation, repository, zip archive
│   │   ├── reconcile/          # Boot-time orphan temp-dir sweep
│   │   ├── router.rs           # routes + middleware (request-id, trace, CORS, timeout)
│   │   ├── routes/             # HTTP handlers: health, packages, sessions
│   │   ├── sessions/           # Session lifecycle service
│   │   └── state.rs            # AppState { config, db, packages, sessions, authz }
│   └── tests/
│       ├── auth_api.rs         # JWT auth integration tests
│       ├── health.rs           # router-level tests (no Docker)
│       ├── packages_api.rs     # Package CRUD + CORS integration tests
│       └── packages_archive.rs # Zip archive upload integration tests
└── fkst-worker/
    ├── Dockerfile             # engine-laden worker image (see §Docker images)
    └── src/
        └── main.rs             # skeleton: log role, await SIGTERM
```

> **Shared vs. control-plane:** the role-neutral types (models, NyxID client,
> LLM client, Ornn types, vault model) live under `backend/fkst-shared/src/`.
> The Axum handlers, MongoDB wiring, and all business logic remain in
> `backend/fkst-control-plane/src/`.

## Docker images

Each deployable has its own Dockerfile under its crate directory; both build
from the **repo root** as context (the dependency-cache layering needs every
workspace crate manifest + `Cargo.lock`, which live under `backend/`):

| Dockerfile | Image | Contents | ENTRYPOINT |
|------------|-------|----------|------------|
| `backend/fkst-worker/Dockerfile` | `fkst-worker` | engine-laden (built from `backend/engine.ref`); carries the engine, codex, nyxid CLI, and the runtime volume. Requires `--build-arg FKST_SUBSTRATE_REF`. | `fkst-worker` |
| `backend/fkst-control-plane/Dockerfile` | `fkst-control-plane` | slim (only `ca-certificates`, no engine, runs as uid 10001). No `FKST_SUBSTRATE_REF` needed. | `fkst-control-plane` |

**Local builds:**

```sh
# Worker image (engine-laden)
docker build -f backend/fkst-worker/Dockerfile \
  --build-arg FKST_SUBSTRATE_REF="$(cat backend/engine.ref)" \
  -t fkst-worker:dev .

# Control-plane image (slim)
docker build -f backend/fkst-control-plane/Dockerfile \
  -t fkst-control-plane:dev .
```

> **Transitional caveat (until #151).** Engine execution currently still lives
> in `fkst-control-plane`, so the control-plane process still spawns engine
> children at runtime. Until issue #151 moves engine execution to the worker,
> run the **controller** Deployment from the engine-laden `fkst-worker` image
> (with a `command:` override pointing at the `fkst-control-plane` binary the
> controller image also carries) and keep the runtime volume in place. The slim
> `fkst-control-plane` image builds cleanly but is not yet the controller's
> production home. Once #151 lands, the controller switches to the slim
> `fkst-control-plane` image and drops the volume.
