# FKST Local QA Host

This directory is the single, independently buildable product boundary for
local QA inside `fkst-hosted`. The human-facing trusted-input MVP process is
**Local QA Host**. Its application boundary is `host/`, with Rust package and
executable name `fkst-local-qa-host`.

The Host currently implements only a fail-closed startup/configuration seam.
Because no configuration is supported yet, zero-argument startup exits with
status `1`, writes `fkst-local-qa-host: no supported configuration` to stderr,
and performs no runtime side effects.

The launcher, supervisor, guest agent, Secret Broker, and TypeScript workers
remain intentionally inert. They establish repository and build boundaries
only and remain reserved for a separately reviewed future hardened Local QA
Runtime Profile.

The Host defines no endpoint, persistence, runner or worker coordination,
browser control, Compose or VM execution, Secrets, Evidence, Artifacts, NyxID,
Hosted transport, cleanup, or recovery behavior. It does not claim hardened
Runtime compatibility. Those capabilities require separate issues and review.
See the
[Local QA Host MVP design](../../docs/local-qa-runtime/mvp/LOCAL-QA-HOST-DESIGN.zh-CN.md)
for the target trusted-input design; its presence does not claim implementation.

Verify with the locked Cargo commands, the workers `build` and `typecheck`
scripts, and `bash tests/scaffold-structure.sh`.
