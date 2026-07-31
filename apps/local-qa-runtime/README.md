# FKST Local QA Host

This directory is the single, independently buildable product boundary for
local QA inside `fkst-hosted`. The human-facing trusted-input MVP process is
**Local QA Host**. Its approved future implementation location is `host/`, with
Rust package and executable name `fkst-local-qa-host`; that crate does not exist
yet.

The current Rust binaries and TypeScript workers package are intentionally
inert. They establish repository and build boundaries only; they do not launch
processes, VMs, browsers, listeners, proxies, projects, or Secret helpers. The
existing launcher, supervisor, guest agent, Secret Broker, and workers remain
reserved for a separately reviewed future hardened Local QA Runtime Profile.

This scaffold defines no protocol, ledger, grant, lease, fencing rule, guest
channel, artifact format, installation, cleanup, recovery, or update behavior.
Those capabilities require separate issues and review. See the
[Local QA Host MVP design](../../docs/local-qa-runtime/mvp/LOCAL-QA-HOST-DESIGN.zh-CN.md)
for the target trusted-input design; its presence does not claim implementation.

Verify with the locked Cargo commands, the workers `build` and `typecheck`
scripts, and `bash tests/scaffold-structure.sh`.
