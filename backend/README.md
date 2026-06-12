# fkst-hosted-api

`fkst-hosted-api` is the Rust (Axum) HTTP backend of **fkst-hosted**. It stores
fkst lua packages in MongoDB and runs [fkst-substrate](https://github.com/ChronoAIProject/fkst-substrate)
sessions on behalf of users. This is the v1 scope; this README covers **local
development** only.

## Prerequisites

- **Rust** (stable toolchain — pinned by `rust-toolchain.toml`, which also pulls in `rustfmt` and `clippy`)
- **Docker** (for the local MongoDB and the integration tests)

## Local dev quickstart

1. Start MongoDB 7 (data persists in the named volume `fkst_mongo_data`):

   ```sh
   docker compose -f backend/docker-compose.yml up -d
   ```

2. Run the API (from `backend/`). `MONGODB_URI` is required — the server
   fails closed at startup if it is missing or Mongo is unreachable:

   ```sh
   MONGODB_URI="mongodb://localhost:27017" cargo run -p fkst-hosted-api
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
MONGODB_URI="mongodb://localhost:27018" cargo run -p fkst-hosted-api
```

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
| `FKST_HOSTED_LOG_LEVEL` | no | `info` | `tracing-subscriber` `EnvFilter` directive (e.g. `debug`, `fkst_hosted_api=debug`). An invalid directive falls back to `info` with a warning. Logs are JSON. |
| `FKST_HOSTED_REQUEST_TIMEOUT_SECS` | no | `30` | Per-request timeout in seconds (`408 Request Timeout` on expiry). Must be ≥ 1; `0` is rejected at startup. |
| `FKST_AUTH_ENABLED` | no | `true` | Enable NyxID JWT authentication. Set to `"false"` for local dev (all routes open, extractor yields dev context). Default is fail-closed: auth is on unless explicitly disabled. |
| `FKST_AUTH_NYXID_BASE_URL` | when auth enabled | — | NyxID base URL for the JWKS endpoint (e.g. `https://nyxid.example.com`). Trailing `/` is trimmed. Required when `FKST_AUTH_ENABLED=true`. |
| `FKST_AUTH_ISSUER` | no | `nyxid` | Expected JWT `iss` claim. |
| `FKST_AUTH_AUDIENCE` | no | same as base URL | Expected JWT `aud` claim. Defaults to the (trimmed) `FKST_AUTH_NYXID_BASE_URL`. |
| `FKST_AUTH_JWKS_CACHE_TTL_SECS` | no | `300` | JWKS cache TTL in seconds. Must be ≥ 1; `0` is rejected at startup. After TTL expiry, stale keys are served if the refresh fetch fails. |
| `NYXID_CLIENT_ID` | no | — | NyxID service-account client ID for org APIs (e.g. `sa_…`). Both-or-neither with `NYXID_CLIENT_SECRET`. Without both, org features degrade gracefully (owner-only authorization). |
| `NYXID_CLIENT_SECRET` | no | — | NyxID service-account client secret (SECRET). Both-or-neither with `NYXID_CLIENT_ID`. |
| `FKST_NYXID_ORG_CACHE_TTL_SECS` | no | `30` | TTL in seconds for NyxID org-role and user-orgs caches. Controls how stale org membership data may be. Must be ≥ 1; `0` is rejected at startup. |

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

## Testing

```sh
cargo test --workspace
```

- Unit tests and the router tests (`tests/health.rs`) run anywhere — no
  Docker, no real TCP bind.
- The Mongo integration tests (`tests/persistence.rs`) start an ephemeral
  MongoDB container via `testcontainers` and **skip cleanly when Docker is
  absent**, so the suite stays green on Docker-less runners.

## Project layout

```
backend/
├── Cargo.toml              # cargo workspace (members: fkst-hosted-api)
├── docker-compose.yml      # local dev MongoDB 7 (named volume fkst_mongo_data)
├── Dockerfile
├── rust-toolchain.toml     # stable + rustfmt + clippy
└── fkst-hosted-api/
    ├── src/
    │   ├── main.rs         # entrypoint: JSON tracing, config, Mongo connect, graceful shutdown
    │   ├── lib.rs          # module exports (binary + integration tests share them)
    │   ├── config.rs       # typed Config from env (FKST_HOSTED_* + MONGODB_*)
    │   ├── db.rs           # Db handle: typed collections, ping, idempotent indexes, URI redaction
    │   ├── error.rs        # AppError -> canonical JSON error envelope
    │   ├── models.rs       # BSON document models (packages, sessions, leases)
    │   ├── router.rs       # routes + middleware (request-id, trace, CORS, timeout)
    │   ├── state.rs        # AppState { config, db }
    │   └── routes/
    │       └── health.rs   # GET /health, GET /api/v1/health
    └── tests/
        ├── health.rs       # router-level tests (no Docker)
        └── persistence.rs  # testcontainers Mongo tests (skip without Docker)
```
