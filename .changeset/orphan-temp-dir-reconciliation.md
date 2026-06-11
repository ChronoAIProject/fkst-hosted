---
"fkst-hosted": minor
---

Boot-time orphan temp-dir reconciliation (issue #26, reduced scope). A new `reconcile` module sweeps engine temp dirs (`fkst-rt-*` / `fkst-pkg-*`) under `FKST_HOSTED_ENGINE_TEMP_ROOT` that a hard-killed (`SIGKILL` / OOM) previous incarnation of the same pod leaked — the only orphan class that exists in landed v1. The sweep is fenced against the `runtime_dir` values of non-terminal sessions in Mongo and an mtime safety threshold so an in-flight session's dir is never removed, isolates per-entry removal failures (one un-removable dir never aborts the pass), and runs at startup fail-open (a sweep error logs a warning and never blocks the server). New config: `FKST_HOSTED_RECONCILE_MIN_AGE_SECS` (default 300) and `FKST_HOSTED_RECONCILE_DRY_RUN` (default false).

Reduced scope: the original issue targeted cleaning stale git worktrees and candidate branches in a shared host repo after takeover, but that surface does not exist in v1 — the #17 spike proved packages materialize as plain temp dirs (no git, no worktrees, no candidate branches) and #25's journaling is GitHub-API-only, so a dead pod's filesystem dies with the pod. The worktree/branch reconciler should be reintroduced if and when a shared host-repo surface ever lands.
