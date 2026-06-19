# github-external-pr-intake

`github-external-pr-intake` is a flat Ports-and-Adapters boundary for one job: turn an open
third-party GitHub pull request into one normal GitHub issue that `github-devloop` can already
process.

The package exists because the established boundary is an Anti-Corruption Layer plus an
Idempotent Consumer. External contributor PRs are untrusted PR facts, while `github-devloop` is an
issue-driven implementation, review, and merge workflow. The bridge therefore owns only scheduled
PR detection and idempotent issue materialization; `github-devloop` remains unchanged and keeps all
implementation, review, CI, and merge authority.

## Why This Is Not `github-proxy`

`github-proxy` is the GitHub protocol adapter. Its `github_poll` department polls issues and PRs as
generic GitHub entity facts, and its issue intake path is intentionally issue-shaped. Teaching it
to decide which PRs are external, claim those PRs, create bridge issues, and write
`external-pr-bridge:v1` markers would add domain policy and lifecycle ownership to the protocol
adapter.

That would collapse two responsibilities:

- `github-proxy`: observe GitHub entities and execute requested GitHub effects.
- `github-external-pr-intake`: select external PR candidates and create exactly one bridge issue
  for each accepted PR.

Keeping the bridge outside `github-proxy` follows Single Responsibility and keeps the protocol
adapter open for reuse by packages that do not want external PR intake.

## Why This Is Not Manual Intake

A manual or no-op issue template can represent the final bridge issue after a human notices a PR,
but it cannot perform the required autonomous job:

- scheduled detection of newly opened external PRs;
- filtering out managed bot PRs and `devloop/` heads;
- cross-instance single-winner coordination with `with_lock(core.bridge_lock_key(...))`;
- durable deduplication through trusted `external-pr-bridge:v1` markers and bridge issue search.

Manual intake is therefore a different operating mode, not a replacement for this package's
scheduled adapter.

## Contract

The package consumes `external_pr_scan` and `external_pr_candidate`, produces only
`external_pr_candidate`, and writes no `github-devloop` state. Its durable bridge fact is the
trusted bot-authored `external-pr-bridge:v1` marker plus the matching open bridge issue. Reliable
payloads carry `source_ref` and small control fields; PR content stays at GitHub and is re-derived
from `external:<repo>#pr/<number>`.

⟦AI:FKST⟧
