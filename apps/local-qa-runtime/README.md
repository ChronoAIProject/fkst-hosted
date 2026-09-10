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
and exits. Default production does not invoke this Worker. The non-default
`mvp0-browser-test` Host feature supplies one reviewed MVP-0A acceptance peer
that owns an explicitly supplied Node executable, prebuilt uncommitted Worker
bundle, explicit system Chrome executable, bounded pipes, process group, timeout,
clean EOF, and process reap.

The worker never discovers or launches Chrome, opens network or filesystem
resources, creates profiles, downloads, or child processes, persists Evidence,
or owns Host cleanup. Browser output acquisition, screenshot production,
reference digesting, and storage are responsibilities of the protocol peer, not
worker policy.

The Rust `evidence-stager/` library owns bounded Local Evidence filesystem
effects. It accepts validated Evidence identities and bytes, derives confined
paths beneath an injected quarantine root, publishes through a synced
same-directory temporary file and hard-link handoff, returns contract-validated
object metadata and canonical digest-bound references, and verifies reopened
published bytes. Sanitized observations are canonical JSON under the same
run/attempt root but outside `evidence/`, so the fixed screenshot and runner-log
quota remains exactly two Evidence objects. The feature-gated MVP-0A Host path
stages and reloads all three artifacts; default production does not.

The approved partial #6146 increment adds `stage_fixed_json_export` and
`read_fixed_json_export` to this library only. The sole immutable local built-in
profile accepts the existing exact `LocalSanitizedObservation`, preserves its
equal loopback fixture URLs and fixed selector/READY values, rejects unknown or
duplicate fields before any publication, and bounds raw input and canonical
output to 64 KiB each. `StagedEvidence` remains local-only; raw logs and PNG are
never eligible. This profile does not implement the signed hardened policy or
Hosted-frozen authority, and does not activate production Browser or upload.

Fixed JSON uses `fixed-json/<run>/<attempt>/{raw,export}/` beneath an absolute,
non-symlink, Host-owned root. Raw bytes and a receipt-digest anchor precede
canonical output and the final durable receipt. Only complete revalidated
records yield an opaque handle; replay preserves the original receipt timestamp.
Each attempt has at most two fixed JSON objects, with the existing 2 MiB ceiling
counting raw, output and receipt metadata together. `fixed_json_status` and
`cleanup_fixed_json` expose exact run/attempt/namespace ownership for #6159;
status counts physical files, including interrupted publication, without granting
eligibility. Raw cleanup revokes existing handles; export cleanup is independent.
These hooks do not coordinate resources or release execution slots. The local
integrity checks assume Host-owned storage, not protection from a privileged
writer replacing the entire store. Original #6146 remains open for its unimplemented
provider, full redaction-policy and PNG requirements.

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

The Rust Browser adapter under `browser-adapter/` owns one fixed loopback
fixture, a fresh Chrome process group, a temporary profile, and a separate
temporary downloads directory. Its prepared-session API accepts an explicit
Chrome path, exposes the randomized fixture URL before Worker invocation,
permits exactly one observation, and supports explicit close. It returns the
exact final URL, rendered fixed-element text, and validated bounded `1280x720`
PNG without evaluating pass/fail. The compatibility wrapper still uses the fixed
system-Chrome allowlist and asserts only that the production fixture rendered
`READY`. Explicit close is the primary cleanup path; the 15-second operation
deadline and `Drop` safety net remain fallback containment.

The feature-gated MVP-0A path seeds one executable v1 test row, runs the exact
Browser executor selection through the existing registry and coordinator, stages
one sanitized observation plus screenshot and runner-log Evidence, explicitly
closes Browser ownership, reaps the Worker, and persists the existing terminal
Journal outcome. Restart after durable completion proves those effects are not
repeated. This is infrastructure execution only: it is not enabled in default
production, does not make v2 rows claimable, and does not claim Testing Packages
`CaseResultSet` authority.

The Host contains reusable local lifecycle drivers for an immutable Source payload,
a revalidated read-only Source cache, fresh Runtime-derived per-Run workspaces,
exact Environment ownership status/stop receipts, and bounded loopback readiness
receipts. Source acquisition requires an explicit `TrustedLocalSourceBinding`
supplied by the trusted local embedding independently of the incoming reference.
It binds the complete reference kind/id/schema/digest, a separate source object ID,
the expected raw-byte SHA-256, expected immutable revision, and mandatory nonempty
source provider scope and identity. Every incoming reference must match that binding
and the pinned generic reference shape. A schema string such as `qa.source/v1`
does not establish a registered executable Source schema or grant authority.

The driver hashes actual raw bytes and compares the provider's declared object,
revision, scope and identity with those expectations. Reference digests and
`ObjectDigest` revision labels remain distinct from raw-byte digests. Matching
Git commit or snapshot/tree labels is local consistency, not proof of Git contents,
a reconstructed tree, or authenticated provenance. No signature, issuer, transport,
production Source lease, or new contract schema is implemented here.

Byte storage remains addressed by raw digest, with explicit v2 byte metadata and
separate receipts for complete source bindings. Exact binding replay revalidates
both metadata and raw bytes without re-contacting the provider. A different binding
must freshly acquire and match all expected facts before sharing the same bytes.
Existing v1 metadata, corrupt or partial records, and an existing workspace's missing
source receipt are unavailable; the manager does not delete, reconstruct or silently
upgrade them. A missing receipt for a new binding never suffices to authorize reuse.
General cache publication/recovery and garbage collection remain a separate unit.

The bounded workspace ownership driver uses the Host Journal's v9 serialization
compatibility fence, so older v8 readers reject databases containing the new format.
Migration advances only the version; it preserves existing v8 rows byte-for-byte
without inventing source bindings. Missing `source_binding` deserializes as `None`
and is omitted on serialization. State transitions retain existing intent/resource
JSON bytes, including legacy formatting. Legacy prepare is denied before acquisition;
independent recorded workspace ownership still permits safe recover/status/stop.
New workspace intent includes the full binding and rejects changes within the same
Run/generation before acquisition or workspace effects. `SourceWorkspaceManager::new` takes an open persistent
Journal and an explicit local provider scope with its additional writable roots.
Its database parent must be disjoint from cache, workspace and declared provider
writable trees. On Unix, `Journal::open` optionally observes the main file and parent
chain identities immediately after SQLite opens and before WAL setup/migration.
The manager compares that observation with pinned files before executing manager
SQL, and rechecks pinned database, sidecar and parent attachment around Journal
access. Missing identity capture blocks manager construction. Existing Journal
callers retain their WAL/migration behavior; memory and anonymous databases still
cannot satisfy that existing WAL contract, and workspace management remains
unsupported on non-Unix platforms.

This assumes Host-owned provisioning remains stable across SQLite open and the
subsequent metadata observation, and adapters accurately declare writable roots.
The observation is not an atomic binding to SQLite's internal file descriptor;
the available safe API does not provide that guarantee. A concurrent privileged
provisioner must not replace storage during open. The checks detect later stale
connections and attachment changes; they do not isolate a provider with arbitrary
same-user access to the machine.

Workspace intent snapshots the current internally supplied source facts, initial
deadline, Run/generation, derived location and provider scope. Conditional Journal
transitions commit directory/create/stop attempts before callbacks. Exact provider
discovery can recover creation interrupted before binding; absent after an attempt,
unknown and conflicting discovery never authorize automatic recreation. Opaque
handles are checked against the durable record, and writable markers supply only
diagnostic consistency. Replay requires an observed active provider. Recorded stop
retains directory identity and any pending filesystem cleanup; a missing marker
permits only removal of the same recorded empty directory. These records do not
release the global execution slot or produce a global CleanupReceipt.

`recover` reconciles an existing workspace key without admitting creation, including
after its initial deadline. It returns ownership for status/stop, not permission to
use a stopped resource. A directory creation interrupted before identity persistence,
or unmarked partial data after provider stop, remains blocked for explicit recovery.
These fake-provider tests establish local consistency only; they do not authenticate
SourceObject leases or provide real Compose acceptance or production activation.

These drivers are not wired into production admission. The pinned executable
contracts still expose only generic `DigestBoundReferenceV2` values and do not
provide the approved `SourceObjectLease` binding, controlled Environment
Profile-to-provider projection, or readiness receipt mapping required before
real Source or Compose effects. The Host therefore keeps that boundary
fail-closed rather than treating repository/commit equality, profile digests,
caller configuration, or fake providers as execution authority.

The following capabilities remain explicitly deferred:

- production Browser executor registration, v2 claiming, and persisted v2
  selection cutover;
- journal receipt or Evidence-reference authority beyond the local staging
  owner;
- crash-after-effect uncertainty and restart-to-`lost` reconciliation;
- NyxID and Hosted transport or authentication;
- production Source/Compose activation and Secrets;
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
cargo clippy -p fkst-local-qa-host --features mvp0-browser-test --all-targets --locked -- -D warnings
cargo build --workspace --locked
cargo test --workspace --locked
```

After building `workers/dist/worker-main.js`, Linux can run the ignored real
walking skeleton with explicit executables:

```bash
FKST_LOCAL_QA_NODE="$(command -v node)" \
FKST_LOCAL_QA_CHROME="$(command -v google-chrome)" \
cargo test -p fkst-local-qa-host \
  --features mvp0-browser-test \
  --locked browser_worker_walking_skeleton \
  -- --ignored --nocapture
```

`workers/dist/` is generated and remains uncommitted. The staging root is also
explicitly supplied by the test harness; no environment variable or production
configuration activates the Browser executor.

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
