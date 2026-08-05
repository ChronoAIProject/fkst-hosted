# workflow-runner

Executes a **scheduled workflow** inside the session pod and reports the run back
to the schedule's definition issue.

This is the pod-side half of the scheduled-workflow capability. The control-plane
half — the clock, the dispatch, the watchdog, the API, the dashboard — lives in
`fkst-hosted@develop` and is already shipped.

## How a run reaches this package

1. A user opens an issue labelled `fkst-scheduled-workflow` naming a workflow, a
   run mode, and arguments, assigned to the session's creator.
2. The control-plane clock evaluates it each reconcile. On a due slot it creates a
   **run issue**: App-authored, carrying the session's work label, assigned to
   exactly the creator, and carrying an `fkst-cron-dispatch:v1` marker plus a
   fenced `toml` block of arguments.
3. That run issue is an ordinary routed work issue, so it wakes the session
   through the gate that already exists — no new spawn path.
4. This package's boot-once raiser fires ~30 s after pod start, finds the run
   issue, executes the workflow's steps, and posts one `fkst-cron-run:v1` record.
5. The control plane sees the terminal record and releases the schedule.

## The workflow definition

`.fkst/workflows/<id>.toml` in the target repository:

```toml
description = "Source candidates, score them, publish the accepted subset."

[[step]]
id = "scrape"
kind = "run"
command = ["python3", ".fkst/workflows/sourcing/scrape.py", "--role", "{{ role }}"]
timeout_secs = 900

[[step]]
id = "score"
kind = "task"
prompt = "Score every entry in candidates.json against: {{ role }}. Write scored.json."
timeout_secs = 1800

[[step]]
id = "publish"
kind = "run"
command = ["python3", ".fkst/workflows/sourcing/publish.py", "--min-score", "{{ min_score }}"]
```

Two step kinds:

- **`run`** — deterministic. `command` is an **argv array**, executed in the
  session workspace with the session's git credentials available, so a step may
  commit and push. Non-zero exit fails the step.
- **`task`** — agentic. `prompt` is handed to codex, honouring the session's own
  `### Engine Config` model and effort exactly as any other session work.

Steps run **in declared order**. A failed step fails the run and later steps do
not execute — half a pipeline is worse than none, because a sourcing workflow
whose scrape failed and whose publish ran anyway would publish an empty result
over a good one. Skipped steps are still recorded, so the history shows how far
the run got.

`timeout_secs` defaults to 900.

## Arguments are data, never shell text

`{{ name }}` placeholders are substituted into **one argv element** or into a
prompt. The runner quotes each argv element separately, so a value containing
`;`, a quote, or a newline is an ordinary argument rather than syntax.

A placeholder with no supplied argument is an **error**, not an empty string:
running a scrape with a blank search term would produce a plausible, wrong result
rather than a visible failure.

## Secrets

Arguments live on a public issue body, and the control plane refuses a
credential-shaped value outright. Secrets reach a step **only** through a named
environment profile, referenced by key name from the workflow definition or read
from the environment by the step's own script. Never put a token in
`### Arguments`, in the definition file, or in a step's argv.

## What this package deliberately does NOT do

- **It writes no label.** The control plane is the single writer of every
  `fkst-cron-*` label; that is what makes the overlap rule and the watchdog
  trustworthy. A label written from here would race the reconciler for state it
  does not own. `run_report_test.lua` pins this.
- **It declares no `[github] work_labels`.** A scheduled run is woken by the run
  issue, which already carries the session's own label. Declaring one here would
  add it to `FKST_SESSION_WORK_LABEL` and expose the session to an unrelated
  second intake.
- **It does not poll.** The pod exists only because a run issue woke the session,
  so a polling raiser would be redundant and a standing cost.
- **It does not use the blueprint workflow engine.** `libraries/workflow/engine`
  materializes every step as a child issue worked into a pull request, and its
  content kinds are closed to `{static, generated}`. These workloads produce no
  code and no PR, so that engine would file an issue per step for work that has
  nothing to review.
- **It never reads `FKST_SESSION_PACKAGE_ENV_JSON`.** That is the author-written
  `### Package Env` blob; a trigger author must not be able to forge which
  workflow runs or what it executes.

## The definition-file reader

`runner/toml.lua` accepts an **enumerated subset** of TOML — `[[step]]` headers,
basic strings, non-negative integers, booleans, and single-line string arrays —
and refuses everything else with a line number. That is deliberate: a definition
using a TOML feature the reader does not implement gets a clear
"unsupported syntax at line N" rather than being silently misread, and a misread
definition runs the wrong commands.

## Composing it

Reference `manifests/scheduled-workflows.json`, or add
`ChronoAIProject/fkst-hosted@packages:packages/workflow-runner` to a trigger's
`### Packages` alongside `github-proxy` and `github-comment-effect`.
