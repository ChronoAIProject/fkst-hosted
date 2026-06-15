# Security Policy

## Supported Scope

Security reports for `fkst-packages` should cover this repository's Lua packages, repository scripts,
test harnesses, documentation that affects operation, and package-level GitHub automation behavior.

Engine runtime, SDK primitive, delivery-store, sandbox, or Rust implementation issues belong in the
separate `fkst-substrate` repository unless the vulnerable behavior is caused by package code in this
repository.

中文补注：本仓安全范围是 package 行为层；引擎运行时与 Rust 实现问题应归 `fkst-substrate`。

## Reporting a Vulnerability

Use GitHub private vulnerability reporting for this repository when available:

```text
https://github.com/ChronoAIProject/fkst-packages/security/advisories/new
```

If private vulnerability reporting is unavailable, open a minimal public issue asking the maintainers
for a private reporting channel. Do not include exploit details, secrets, tokens, private logs, or
proof-of-concept payloads in the public issue.

Include enough information for maintainers to reproduce and assess the issue privately:

- affected package, script, or workflow;
- the security impact;
- reproduction steps or a minimal proof of concept;
- whether GitHub write posture, credentials, branch protection, or external command execution is
  involved;
- any known mitigations.

## Handling Expectations

Maintainers should acknowledge valid private reports, keep sensitive details out of public issues
until a fix is available, and land fixes through the normal issue to PR to review to merge workflow.
Do not use security reports as permission to modify program state, GitHub labels, comments, branches,
or PRs outside the repository's normal guarded paths.

⟦AI:FKST⟧
