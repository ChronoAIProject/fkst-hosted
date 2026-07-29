# FKST Local QA Runtime

This directory owns the user-facing Local QA Runtime as an independently
buildable product boundary inside `fkst-hosted`.

The current Rust binaries and TypeScript workers package are intentionally
inert. They establish repository and build boundaries only; they do not launch
processes, VMs, browsers, listeners, proxies, projects, or Secret helpers.

This scaffold defines no protocol, ledger, grant, lease, fencing rule, guest
channel, artifact format, installation, cleanup, recovery, or update behavior.
Those capabilities require separate issues and review.

Verify with the locked Cargo commands, the workers `build` and `typecheck`
scripts, and `bash tests/scaffold-structure.sh`.
