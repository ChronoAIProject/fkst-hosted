# `github-autochrono` Composed Package

This is a **composed package** (adapter / wiring layer) that combines `github-proxy` and
`autochrono` while the two packages remain unaware of each other. `github-proxy` only publishes
GitHub entity changes and consumes comment requests; `autochrono` only consumes its own `issue`
contract and produces its own `reply` contract. This package's glue departments are the only layer
that references both `github-proxy.*` and `autochrono.*` queues, keeping that coupling centralized.

`fkst.toml` `[event_deps]` declares the sibling packages it composes (`github-proxy` and
`autochrono`) as the only source used by the standard test entrypoint to assemble composed
conformance. Because the glue departments reference cross-package namespaces, this package does not
run single-root conformance; it is valid only inside the composed graph.

Flow:

```text
github-proxy.github_entity_changed
  -> autochrono.issue
  -> consensus.proposal
  -> consensus.consensus_reached
  -> autochrono.reply
  -> github-proxy.github_issue_comment_request
```

`departments/inbound_glue` maps only GitHub issue events to `autochrono.issue.v1` and ignores PRs.
`autochrono` maps issues to `consensus.proposal.v1`; `consensus` produces
`consensus.consensus_reached.v1`; only an approve result continues to `autochrono.reply.v1`.
`departments/outbound_glue` maps `autochrono.reply.v1` to `github_issue_comment_request`, preserving
`issue_number`, `body`, `dedup_key`, and `source_ref`. `core.lua` contains only pure mapping
functions, and `tests/core_test.lua` tests those functions without depending on the composed graph or
runtime `PATH`.

Tests (standard entrypoints):

```sh
scripts/run.sh test            # all packages: flat single-root + composed tests + composed conformance
scripts/run.sh test-composed   # composed graph conformance only, recursively unioning github-proxy + autochrono + consensus + this package from [event_deps]
```

⟦AI:FKST⟧
