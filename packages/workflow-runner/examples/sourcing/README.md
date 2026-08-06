# Example: an external-sourcing pipeline

A reference **shape** for a three-step scheduled workflow that queries a
third-party API, applies model judgment to each result, and publishes the
accepted subset — while keeping a de-duplication ledger across runs.

It is deliberately a poor fit for the blueprint workflow engine: it produces **no
code and no pull request**, which is exactly why the linear `run`/`task` executor
exists.

> **Content boundary.** This is the shape and the contract, not an instance. The
> search parameters, judgment criteria, destination identifiers, and
> credential-broker service names belong in the operator's own copy — supplied
> through workflow arguments and an environment profile, never committed here.

## Installing it

Copy `workflow.toml` to `.fkst/workflows/<your-id>.toml` in the target repository
and the three scripts to `.fkst/workflows/sourcing/`. Then open a
`fkst-scheduled-workflow` issue naming `<your-id>`, assigned to the session
creator:

```markdown
### Workflow
<your-id>

### Run Mode
cron: 0 1 * * 1-5

### Arguments
role: <your search parameter>
min_score: 6
```

## Why each step is shaped the way it is

**Step 1 — `scrape` (`run`, deterministic).** Queries the API across a
parameterized set of terms, drops entries already in the ledger, enriches the
remainder, and writes `candidates.json`.

It must handle pagination, back off on rate-limit responses, and tolerate an
individual enrichment call failing **without failing the run** — one flaky
enrichment out of two hundred is not a reason to lose the other 199. A whole-API
outage IS a reason to fail, and does: the run records a failed status, the ledger
is untouched, and the next slot proceeds normally.

**Step 2 — `score` (`task`, agentic).** Scores each entry against criteria passed
as a workflow argument, emitting `scored.json`.

It must parse **defensively**: a model asked for JSON will sometimes wrap it in
prose or a fenced block. The failure mode to avoid is silently scoring everything
zero — that would publish nothing while reporting success. `parse_scored.py`
extracts the payload from prose or a fence and **fails the step loudly** when it
cannot, which surfaces as a failed run with the step named.

**Step 3 — `publish` (`run`, deterministic).** Filters by `min_score`, POSTs each
accepted entry through the credential broker, appends accepted ids to the ledger,
and commits the ledger back to the repository.

## The ledger is what makes this stateless

Cross-run state is a **committed file in the target repository**
(`.fkst/workflows/sourcing/ledger.json`), never a control-plane store. That is
the same principle the whole capability rests on: everything durable lives in
GitHub.

Two properties it must have:

- **Bounded growth.** Retain the most recent N ids; an unbounded ledger becomes a
  file nobody can review and eventually a slow clone.
- **Idempotent commit and publish.** Re-running the same slot must not
  double-publish. Rows are keyed by entry id, so a partial step-3 failure is
  safely retryable: the entries that landed are already in the ledger, and the
  ones that did not are retried on the next run.

## Credentials

The external API token and the credential-broker key are delivered **only** as
environment-profile secrets and read from the process environment by the scripts.

No token appears in the definition, in the work issue, in the repository, or in
any log line. The runner's own
`test_a_credential_shaped_value_never_reaches_a_run_record` pins the last of
those: output tails are truncated and details are sanitized, and neither may
carry a credential-shaped substring into the run history.
