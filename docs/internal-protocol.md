# Internal controller ↔ worker protocol

The authoritative contract for the fleet-internal protocol the **controller**
(`fkst-control-plane`) and the **workers** (`fkst-worker`) speak (issue #134).
The wire types live in the role-neutral crate `fkst-shared` (`protocol`
module); later database-free issues plug behaviour onto this transport without
inventing new wire types.

## Transport & authentication

- Workers connect **up** to the controller over stable Service DNS
  (`CONTROLLER_URL`) and **pull** work; the controller never connects to a
  worker (it has no Kubernetes API access).
- Every internal request carries the shared secret in the
  **`x-fkst-internal-auth`** header (`INTERNAL_AUTH_HEADER`). The controller
  compares it in **constant time**; a missing/wrong header is `401` (logged at
  `warn`, never logging the token). When the controller has no secret
  configured (`FKST_INTERNAL_AUTH_TOKEN` unset) the internal routes are **not
  mounted at all** — the internal surface is closed by default.
- Every request carries an inline `protocol_version` checked against
  `PROTOCOL_VERSION` (currently `1`); a mismatch is `400`.
- Internal routes live at `/internal/v1/*`, **NOT** under `/api/v1` and **NOT**
  behind the NyxID proxy-trust middleware — the public API surface is unchanged.
- Every request body is `deny_unknown_fields` (malformed/extra-field input is
  rejected at the trust boundary).

## Lifecycle state machine

```
Active ──► Draining ──► Terminated
```

`Active` and `Draining` are reported on the heartbeat (`lifecycle_state`).
`Terminated` is not wire-reported: a terminated worker simply stops
heartbeating and is expired by the controller after the liveness TTL
(`FKST_WORKER_LIVENESS_TTL_SECS`, default 30s). The drain **behaviour** is
issue #140; this issue ships only the message vocabulary.

## Routes

| Method · Path | Request | Response | Notes |
|---|---|---|---|
| `POST /internal/v1/register` | `RegisterRequest { worker_id, protocol_version, capacity, engine_temp_root }` | `RegisterResponse { accepted, heartbeat_interval_secs, controller_protocol_version }` | The controller is authoritative for the heartbeat cadence it returns (TTL/3). |
| `POST /internal/v1/heartbeat` | `Heartbeat { worker_id, protocol_version, lifecycle_state, running_sessions, timestamp_unix_ms }` | `HeartbeatResponse { acknowledged, control: [ControlMessage] }` | The controller piggybacks control messages on the answer (pull model). An unknown worker's heartbeat self-heals as a re-register. |
| `POST /internal/v1/pull` | `PullRequest { worker_id, protocol_version, available_capacity }` | `PullResponse { assignments: [WorkAssignment] }` | Always **empty** until claim authority lands (#135). |
| `POST /internal/v1/draining` | `Draining { worker_id, sessions, checkpoint_done }` | `200 {}` | Received + logged; flips the worker's tracked state to `Draining`. Reassignment is #140. |
| `POST /internal/v1/released` | `Released { worker_id, session_id }` | `200 {}` | Drain/handoff acknowledgement (an engine is actually stopped). Logged; reassignment is #140. |

### Control messages (controller → worker)

`ControlMessage` is `#[serde(tag = "type")]`; the only variant today is:

- `StopSession { session_id, reason }` — the worker stops the engine and replies
  with a `Released`. (No engine exists yet — #136 — so the worker logs it and
  sends `Released` to complete the round-trip.)

## Failure handling (worker side)

- **register**: retries transport / 5xx failures forever with bounded
  exponential backoff + per-worker jitter (a worker that cannot reach its
  controller is useless but must not crash-loop tightly). Fails **closed** (exit
  non-zero) on `401` (wrong secret) or an incompatible protocol version.
- **heartbeat / pull**: a failure logs a `warn` and is retried on the next tick.
- On shutdown (SIGTERM / Ctrl-C) the worker stops its loops and sends a
  best-effort final heartbeat with `lifecycle_state = Draining`.

## Controller-side state

The controller keeps an **in-memory** `WorkerRegistry` (no persistence — the
database-free model). A background sweeper expires workers whose last heartbeat
is older than the liveness TTL. There is **no** claim/placement authority yet
(#135) and **no** drain/reassignment logic (#140) — this issue is transport
only.
