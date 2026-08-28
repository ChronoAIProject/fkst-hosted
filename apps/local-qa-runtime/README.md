# FKST Local QA Host

This directory is the single, independently buildable product boundary for
local QA inside `fkst-hosted`. The human-facing trusted-input MVP process is
**Local QA Host**. Its application boundary is `host/`, with Rust package and
executable name `fkst-local-qa-host`.

The Host starts only through the explicit trusted-input command:

```bash
fkst-local-qa-host local-demo --listen <loopback> --database <path>
```

`--listen` accepts only `127.0.0.1:<port>` or `[::1]:<port>`. The Host exposes
exactly these loopback-only routes:

- `GET /v1/health`
- `PUT /v1/runs/{run_id}`
- `GET /v1/runs/{run_id}`
- `GET /v1/runs/{run_id}/events?after={cursor}&limit={limit}`
- `POST /v1/runs/{run_id}:cancel`

The submit route parses and validates strict `qa.local-run-admission/v2`, but
production has no accepted current-claim authority adapter yet and therefore
rejects every new v2 admission before executor resolution or Journal mutation.
The MVP-0 deterministic verifier is available only through the explicit hidden
test serving entry point. Those tests resolve the exact
`qa.local-executor/v1` selection without invoking it and atomically persist the
immutable acceptance bytes, binding, selection, ordered `run.accepted` Event,
and singleton active slot in SQLite journal v6. Exact durable replay does not
re-contact current-claim authority, including after restart; changed keys or
canonical request digests return a mutation-free conflict. `POST` is not an
admission alias, and the former `{"kind":"inert"}` body is rejected.

The snapshot route reads the current durable Run state and latest Event
sequence. The Events route reads Events after the required cursor in ascending
sequence order, subject to the required limit. Both reads remain available
after restart.

Cancellation records durable intent in `cancel_requests` and appends one
ordered `run.cancel_requested` Event. Repeated cancellation does not append a
second Event. Cancellation does not terminate or signal any worker, browser, or
process, and it does not change the accepted Run to a terminal state. Recovery
that reconciles non-terminal Runs to `lost` after restart is not implemented.

Zero-argument and unsupported startup remains fail-closed: it exits with status
`1`, writes `fkst-local-qa-host: no supported configuration` to stderr followed
by one line-feed byte, and performs no runtime side effects. Environment
variables and configuration-looking files do not activate the Host.

The pure TypeScript browser-smoke worker boundary under `workers/` is active
production policy code. It:

- strictly parses one fixed private loopback browser-smoke request;
- consumes only injected controlled browser-session, Evidence-staging, and
  clock ports;
- requires the final URL to equal the requested fixture URL and the observed
  text to exactly equal the fixed expected text;
- validates the local digest-bound reference shapes supplied by its ports;
- generates the fixed `runner.log` through the injected staging port;
- always finalizes the injected session after acquisition; and
- returns a bounded, deterministic serialized result.

The fixed Worker executable walks that policy through the registered bounded
`qa.local-worker-protocol/v1` over stdin/stdout. It accepts one invocation,
performs the seven fixed typed capability exchanges, emits one terminal result,
and exits. The process acceptance harness acts as the Host-shaped peer; the
production Host does not yet invoke this Worker.

The worker never discovers or launches Chrome, opens network or filesystem
resources, creates profiles, downloads, or child processes, persists Evidence,
or owns Host cleanup. Browser output acquisition, screenshot production,
reference digesting, and storage are responsibilities of the protocol peer, not
worker policy.

The Rust `evidence-stager/` library owns bounded Local Evidence filesystem
effects. It accepts validated Evidence identities and bytes, derives confined
paths beneath an injected quarantine root, publishes a synced temporary file by
same-directory atomic rename, returns contract-validated object metadata and a
canonical digest-bound reference, and verifies the reopened published bytes.
The Host and worker do not yet invoke this library.

The Rust Local QA Host API and journal boundary described above is already
activated in `host/`. The launcher, supervisor, guest agent, and Secret Broker
remain intentionally inert hardened-profile shells.

The Testing adapter source is pinned but not activated. Its immutable package
root is
`ChronoAIProject/fkst-packages-testing@ac953ff0bb3f1c909728e66c3968cbb3ed5e3cf1:packages/local-qa-host-adapter`,
with nested platform packages pinned to
`ChronoAIProject/fkst-packages@d4146d7bbdbde9d6fbbee404d5a2e3e4da0fa08c`
and the engine pinned to
`ChronoAIProject/fkst-substrate@e3355b42709f4138613b8238cba34a5ab1161053`.
The reserved canonical schemas are `testing-observation.v1`,
`testing-assertion-result.v1`, `testing-case-result.v2`, and
`testing-case-result-set.v2`. This source-authority pin does not fetch, hydrate,
import, or execute the package graph.

The argument-free Rust Browser adapter under `browser-adapter/` runs one fixed
loopback fixture through the first executable Linux system Chrome found in its
fixed absolute allowlist. Each observation owns a fresh Chrome process group, a
temporary profile, and a separate temporary downloads directory; it returns the
exact final URL, rendered fixed-element text, and validated bounded `1280x720`
PNG without evaluating pass/fail. The legacy fixed smoke wrapper alone asserts
that the production fixture rendered `READY`. Both paths explicitly finalize
all owned resources on success or failure. The 15-second operation deadline and
`Drop` safety net do not expose caller-programmable browser behavior.

The following capabilities remain explicitly deferred:

- Host executor integration of the fixed Browser adapter and Worker protocol;
- Host integration of filesystem Evidence staging and journal Evidence
  references;
- execution terminal outcomes and restart-to-`lost` reconciliation;
- NyxID and Hosted transport or authentication;
- Source, Compose, and Secrets;
- upload, Quality, Report, Publication, and Settlement; and
- hardened VM, egress, or EffectGate claims.

The small Host journal and pure worker policy do not claim hardened Runtime
authority or compatibility. These capabilities require separate issues and
review.
See the
[Local QA Host MVP design](../../docs/local-qa-runtime/mvp/LOCAL-QA-HOST-DESIGN.zh-CN.md)
for the target trusted-input design; its presence does not claim implementation.

From `apps/local-qa-runtime/`, verify the Rust workspace with:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --locked
cargo test --workspace --locked
```

Verify the worker from `apps/local-qa-runtime/workers/` with a clean install:

```bash
npm ci
npm run typecheck
npm run build
npm test
```

Verify the product-boundary scaffold from the repository root with:

```bash
bash apps/local-qa-runtime/tests/scaffold-structure.sh
```
