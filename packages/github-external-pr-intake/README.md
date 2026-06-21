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

Reusing the existing `github-proxy` PR poll plus `github_issue_create_request` effect sink still
leaves the required policy owner missing. `github_poll` can only publish `github_entity_changed`
snapshots, while `github_issue_create` can only execute a complete issue-create request already
prepared by an upstream package. The bridge work is the missing middle: scheduled PR selection,
external-author/head filtering, claim ownership, bridge body construction, and the
`external-pr-bridge:v1` lifecycle. Putting that middle inside `github-proxy` would make the protocol
adapter choose product-domain work instead of merely adapting GitHub.

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
scheduled adapter. A no-op/manual template begins only after a human has already found the PR and
created the issue, so it cannot satisfy the autonomous scheduled-detection requirement.

## Contract

The package consumes `external_pr_scan` and `external_pr_candidate`, produces only
`external_pr_candidate`, and writes no `github-devloop` state. `external_pr_scan` is the reliable,
level-triggered durable source that periodically re-derives open PRs from GitHub. The
`external_pr_candidate` queue is an ephemeral at-most-once activation raised by each scan; if a
candidate handler fails, the next scan raises a fresh activation instead of folding onto a
permanently dead-lettered candidate delivery. Duplicate activations are safe because bridge creation
is idempotent through `with_lock`, trusted `external-pr-bridge:v1` markers, and matching open bridge
issue search. Reliable payloads carry `source_ref` and small control fields; PR content stays at
GitHub and is re-derived from `external:<repo>#pr/<number>`.

## Staleness Window

An external PR becomes bridge-eligible only after it has been open for at least
`FKST_EXTERNAL_PR_BRIDGE_MIN_AGE_SECONDS`. The default is `10800` seconds (3 hours),
which gives human maintainers a priority window before automation creates a bridge issue. The
scan is level-triggered, so younger PRs are skipped and evaluated again on the next scheduled scan;
once the PR age reaches the configured threshold, the normal idempotent bridge path runs.

⟦AI:FKST⟧
