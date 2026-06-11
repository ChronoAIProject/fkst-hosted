# fkst-hosted

**fkst-hosted** is ChronoAI's hosted cloud service for the fkst project: a Rust
([Axum](https://github.com/tokio-rs/axum)) backend with MongoDB that stores fkst
lua **packages** and runs [fkst-substrate](https://github.com/ChronoAIProject/fkst-substrate)
engine **sessions** on behalf of users. It is deployed on Kubernetes.

| Guide | Where |
|-------|-------|
| Local development (backend, MongoDB via Docker Compose, env vars, tests) | [`backend/README.md`](backend/README.md) |
| Kubernetes deployment (Docker Desktop single-replica stack) | [`backend/deploy/k8s/README.md`](backend/deploy/k8s/README.md) |

## Running the end-to-end smoke test

[`scripts/e2e/run-e2e.sh`](scripts/e2e/run-e2e.sh) is a black-box client of the
deployed HTTP API that exercises the v1 happy path: wait for `/health`, create
the `e2e-minimal` package, start a session, poll it to `running`, stop it, poll
it to `stopped`. It exits `0` only when the session reaches `stopped`.

### Prerequisites

- `curl` and `jq` on `PATH`.
- A running fkst-hosted instance to point it at — normally the Kubernetes
  stack from [`backend/deploy/k8s/README.md`](backend/deploy/k8s/README.md).

### Against the Kubernetes stack (primary)

With the stack deployed and healthy, port-forward the Service and run the
script in another terminal:

```sh
kubectl --context docker-desktop -n fkst-hosted port-forward svc/fkst-hosted 8080:80
```

```sh
scripts/e2e/run-e2e.sh
```

### Against a local `cargo run` (Docker Compose fallback)

No cluster needed — start MongoDB via Compose, run the API, run the script
against the default `http://localhost:8080`:

```sh
docker compose -f backend/docker-compose.yml up -d
(cd backend && MONGODB_URI="mongodb://localhost:27017" cargo run -p fkst-hosted-api)
```

```sh
scripts/e2e/run-e2e.sh
```

### Environment variables

All optional; timeouts and the interval are positive-integer seconds.

| Variable | Default | Meaning |
|----------|---------|---------|
| `FKST_HOSTED_BASE_URL` | `http://localhost:8080` | Base URL of the deployment (no trailing slash) |
| `E2E_HEALTH_TIMEOUT` | `60` | Max wait for `GET /health` to answer `{"status":"ok"}` |
| `E2E_START_TIMEOUT` | `120` | Max wait for the session to reach `running` |
| `E2E_STOP_TIMEOUT` | `60` | Max wait for the session to reach `stopped` |
| `E2E_POLL_INTERVAL` | `2` | Sleep between status polls |

### Exit codes

One code per phase, so the failing phase is identifiable from the exit code
alone:

| Code | Phase | Meaning |
|------|-------|---------|
| `0` | — | Success: the session reached `stopped` |
| `1` | usage | Missing `curl`/`jq`, invalid env value, or missing fixture |
| `2` | health | Service unreachable / not `ok` within `E2E_HEALTH_TIMEOUT` |
| `3` | package create | `POST /api/v1/packages` returned anything other than `201`/`409` |
| `4` | session start | Non-`201` response — including `409`: a live session already holds the package lease. **Operator action:** stop the stale session (`POST /api/v1/sessions/{id}/stop`), then re-run |
| `5` | start poll | Session reached `failed`, reported an illegal status, or `E2E_START_TIMEOUT` exceeded waiting for `running` |
| `6` | stop request | `POST /api/v1/sessions/{id}/stop` returned anything other than `202` |
| `7` | stop poll | Session reached `failed`, reported an illegal status, or `E2E_STOP_TIMEOUT` exceeded waiting for `stopped` |

### Reading a failure

- **stdout** carries exactly one thing: the **final session JSON** (status,
  `error` field, runtime details) — fetched even on failure once a session id
  exists, so it is always machine-readable.
- **stderr** carries the progress banners (`[e2e] step N: ...`) and
  diagnostics: the unexpected HTTP code, the response body, any curl errors,
  and — on a timeout — the name and value of the exceeded timeout
  (e.g. `E2E_START_TIMEOUT (120s) exceeded waiting for status 'running'`).

### Re-runs are idempotent

- A `409` on package create means `e2e-minimal` already exists — treated as
  success, the run continues.
- Every run starts a **fresh session**; the previous run's session was stopped
  on the happy path, so the package lease is free. If a prior run died between
  start and stop, the next run fails with exit `4` (lease conflict) and prints
  the stop instruction above.

### Shell compatibility

The script is plain **POSIX `sh`** with a deliberate **zero-pipeline** design:
`pipefail` is not POSIX, so every `curl` writes its body to a temp file and
`jq` reads files directly — correctness never depends on `pipefail`. It is
still enabled opportunistically on shells that support it, as defense in depth.
