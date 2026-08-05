# FKST Local QA Host

This directory is the single, independently buildable product boundary for
local QA inside `fkst-hosted`. The human-facing trusted-input MVP process is
**Local QA Host**. Its application boundary is `host/`, with Rust package and
executable name `fkst-local-qa-host`.

The Host implements one explicit trusted-input walking skeleton:
`fkst-local-qa-host local-demo --listen <loopback-address> --database <path>`.
It accepts only `127.0.0.1:<port>` or `[::1]:<port>`, serves only
`PUT /v1/runs/{run_id}`, accepts only the strict inert JSON body, and journals
the accepted request, Run, and sequence-1 Event atomically in SQLite. The same
idempotency key replays the immutable response after restart; a different key
for the accepted Run returns a mutation-free conflict.

Zero-argument and unsupported startup remains fail-closed: it exits with status
`1`, writes `fkst-local-qa-host: no supported configuration` to stderr followed
by one line-feed byte, and performs no runtime side effects. Environment
variables and configuration-looking files do not activate the Host.

The pure TypeScript Browser smoke worker boundary is activated under `workers/`.
It strictly validates one fixed loopback request, applies the fixed URL and text
assertions through injected ports, stages only its fixed runner log, and returns
deterministically serialized Host-attested evidence references. Raw Browser
output, screenshot bytes, artifact digesting, filesystem persistence, and Host
cleanup do not cross into worker policy.

The Rust Local QA Host API and journal boundary described above is already
activated in `host/`. The launcher, supervisor, guest agent, and Secret Broker
remain intentionally inert hardened-profile shells.

The Host defines no Run lookup, Event lookup, cancellation, execution, runner or
worker coordination, browser control, Compose or VM execution, Secrets,
Evidence, Artifacts, NyxID, Hosted transport, cleanup, or terminal recovery
behavior. Its small journal does not claim hardened Runtime authority or
compatibility. Those capabilities require separate issues and review.

For the worker slice, the Host Browser Controller, real Chrome discovery and
lifecycle, Host-worker process integration, profiles, downloads, filesystem
Evidence persistence, upload, and reporting remain deferred.
See the
[Local QA Host MVP design](../../docs/local-qa-runtime/mvp/LOCAL-QA-HOST-DESIGN.zh-CN.md)
for the target trusted-input design; its presence does not claim implementation.

Verify with the locked Cargo commands, the workers `build`, `typecheck`, and
`test` scripts, and `bash tests/scaffold-structure.sh`.
