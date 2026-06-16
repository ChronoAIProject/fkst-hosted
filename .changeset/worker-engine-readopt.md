---
"fkst-hosted": minor
---

Add OS-truth engine re-adopt + restart recovery, removing the last MongoDB dependency from the runtime-tracking path (#136). Each engine spawn now writes an owner breadcrumb (`owner.json`: pid/pgid + a per-spawn random nonce) and an exit breadcrumb (`exit.json`) into its runtime dir, so a restarted process can re-adopt its still-live engine children purely from OS truth. `RunningSession` gains a `ProcHandle { Owned(Child), Adopted{pid,pgid,run_nonce} }`: an adopted session judges liveness by `is_pid_alive(pid)` + `getpgid(pid)==pgid` + the breadcrumb nonce (the PID-reuse guard), reads its exit from the exit breadcrumb, and stops via signal-by-pgid (there is no `Child` to waitpid). `engine::scan_and_adopt` re-adopts live-owner dirs and age-fence-reaps dead/orphan ones, and `engine::os_truth_live_set` REPLACES the Mongo `live_runtime_dirs` query that fenced the reconcile sweep — `reconcile_orphans` is now a sync, datastore-free `(engine_config, cfg) -> SweepReport`. Externally-observable session status semantics are unchanged.

The OS-truth machinery lives in `fkst-control-plane` next to the engine runner (a 2-reviewer consensus confirmed the `fkst-worker` boundary forbids it from referencing `RunningSession`); the physical engine→worker crate relocation #145 anticipated is tracked separately (#151).
