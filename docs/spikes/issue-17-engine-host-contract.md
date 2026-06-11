# Spike #17 — fkst-framework host contract, empirically pinned

Findings of the engine host-contract spike (issue #17). Every verdict below was produced
empirically inside throwaway containers; verbatim experiment transcripts are cited inline as
`(E<n>-<name>.log)` and were posted with issue #17. Where upstream documentation or canon
hypotheses disagreed with the observed behavior, the logs win.

| Metadata | Value |
|---|---|
| Pinned `fkst-substrate` SHA | `cb072b224d817a8a4b57f92b613cf5f2174d8759` (branch `dev`, read from `/etc/fkst-engine-sha` in the image) (E0-image-selftest.log) |
| Engine image | `fkst-hosted-api:engine-dev` — image id `sha256:e66b47623ace380004b9ee737687872ecf9201ec0370968597ddcc995dddf5b4`, Debian 12 bookworm, runs as `uid=10001(fkst)`; ships the engine binaries + `git 2.39.5` + `bash 5.2.15` (E0-image-selftest.log) |
| Binaries | `/usr/local/bin/fkst-framework` (6,069,624 bytes) — the key binary (`run`, `supervise`, `conformance`, `config`, `test`, `--self-test`); `/usr/local/bin/fkst-supervisor` (2,184,616 bytes) — thin wrapper, **never to be used by the runner** (see Q7) (E0-image-selftest.log) |
| Self-test | `fkst-framework --self-test` → exit `0` (E0-image-selftest.log) |
| Experiment date | 2026-06-11 (UTC) |
| Reproducibility | The full ladder (E2–E11) bundled as one script (`spike.sh`, see Reproduction recipe) was run **twice, each in a fresh `--rm` container**; all **45 normalized verdicts** (exit codes + error strings) were byte-identical across both passes: `IDENTICAL - no hidden host dependency` (REPRO.log) |

> Notation: `supervise` log lines are emitted via `tracing` on stderr with ANSI colors; quoted
> lines below have the color codes stripped but are otherwise verbatim. `exit=124` means the
> process was still running when the experiment's `timeout` harness killed it — i.e. a healthy
> blocking run, not an engine failure.

---

## Q1 — `fkst.env` HostFacts: NOT required for `conformance` or `supervise`

**Verdict: the "fail-closed at conformance" hypothesis is REFUTED.** Both commands start and run
with no `fkst.env` at all. The config registry is a closed 8-entry list — 6 Operational keys with
defaults and exactly 2 HostFacts (`FKST_CANDIDATE_PREFIX`, `FKST_CANDIDATE_FROM_SEP`) with no
default. Resolution order: process env → host `fkst.env` → operational default. HostFacts
fail-closed only when a code path actually *resolves* them (candidate-branch git operations in the
Lua SDK) — never reached by `conformance`/`supervise` startup or by the minimal package's
pipelines.

Registry dump with **no** `fkst.env` — note exit `0` despite both HostFacts missing
(E1-config-registry.log):

```
$ fkst-framework config --project-root $PKG --package-root $PKG
name=queue_capacity env=FKST_QUEUE_CAPACITY kind=Operational type=usize default=16 resolved=16 source=default doc=Default capacity for derived event queues.
name=department_default_stall_window env=FKST_DEPARTMENT_DEFAULT_STALL_WINDOW kind=Operational type=duration-string default=30s resolved=30s source=default doc=Default Department delivery lease window used when M.spec.stall_window is empty.
name=codex_permit_slots env=FKST_CODEX_PERMIT_SLOTS kind=Operational type=usize default=20 resolved=20 source=default doc=Global Codex process permit pool slot count.
name=retry_default_max_attempts env=FKST_RETRY_DEFAULT_MAX_ATTEMPTS kind=Operational type=usize default=5 resolved=5 source=default doc=Default reliable retry max attempts for Departments without a custom value.
name=retry_default_base env=FKST_RETRY_DEFAULT_BASE kind=Operational type=duration-string default=60s resolved=60s source=default doc=Default reliable retry base backoff duration.
name=retry_default_cap env=FKST_RETRY_DEFAULT_CAP kind=Operational type=duration-string default=30m resolved=30m source=default doc=Default reliable retry capped backoff duration.
name=candidate_prefix env=FKST_CANDIDATE_PREFIX kind=HostFact type=string required resolved=missing source=missing doc=Host-owned prefix used for generated candidate branch names.
name=candidate_from_sep env=FKST_CANDIDATE_FROM_SEP kind=HostFact type=string required resolved=missing source=missing doc=Host-owned separator before the parent ref slug in candidate branch names.
exit=0
```

With a 2-key `fkst.env` written, the same dump shows `resolved=candidate/ source=fkst.env` /
`resolved=:: source=fkst.env` (E1-config-registry.log) — `config` is usable as a cheap registry
probe; it does not fail on missing HostFacts.

Empirical ladder on `examples/minimal-package` (E3-hostfacts-ladder.log):

| `fkst.env` state | `conformance` | `supervise` (durable root set) |
|---|---|---|
| file absent | exit **0**, all 5 checks PASS | **starts and processes events** — 10 `framework spawned` in 6 s, zero errors |
| empty file | exit **0** | starts * |
| `FKST_CANDIDATE_PREFIX` only | exit **0** | starts * |
| both keys | exit **0** | starts and processes events |

\* For the empty-file and one-key rungs, `supervise` was run without `FKST_DURABLE_ROOT`; its only
startup failure was `[framework] startup error: FKST_DURABLE_ROOT must be set` — never a HostFact
error — proving HostFacts do not gate startup (E3-hostfacts-ladder.log). The hand-written minimal
package (which has **no** `fkst.env` at all) also ran under `supervise` with 4 spawns
(E5-minimal-tree.log).

- **Operational keys confirmed NOT required** — every run used defaults only (E1-config-registry.log, E3-hostfacts-ladder.log).
- Recommended runner posture: still materialize a 2-key `fkst.env` (`FKST_CANDIDATE_PREFIX=candidate/`, `FKST_CANDIDATE_FROM_SEP=::`) so packages that *do* call the git/candidate SDK do not fail at runtime — but it is **not** a start-up precondition.

## Q2 — git repo: NOT required

**Verdict: NO — neither `conformance` nor `supervise` requires the package/host root to be a git
repo.** The upstream README implication ("host should be a git repo") is refuted for these
commands. Every experiment ran in plain `mktemp -d` directories.

Verbatim (E4-git-ladder.log):

```
+ git -C /tmp/tmp.Q8CvUh9r6k rev-parse --is-inside-work-tree
fatal: not a git repository (or any of the parent directories): .git
exit=128
+ env FKST_RUNTIME_ROOT=... fkst-framework conformance --project-root /tmp/tmp.Q8CvUh9r6k --package-root /tmp/tmp.Q8CvUh9r6k
PASS runtime-layout runtime root accepted: /tmp/tmp.47oX6Zh9jt
PASS project-layout host departments directory readable: /tmp/tmp.Q8CvUh9r6k/departments
PASS graph-scan loaded 2 departments, 1 raisers, 2 queues
PASS department-non-empty host graph contains 2 departments
PASS schema-validation schema validation passed
exit=0
```

`supervise` on the same non-git dir: started and processed events (6 `framework spawned` in 4 s);
`grep -ci git` over the full supervise log = **0** — zero git-related output
(E4-git-ladder.log). **No `git init`, no commit, no `git config user.*` is needed by the runner.**
Git is only reachable via the Lua `sdk_git` API if a package uses it.

## Q3 — git runtime dependency: ship it, but the engine never calls it in this flow

**Verdict: git is not invoked by `conformance`/`supervise`/the minimal run flow at all.** With
`PATH` pointed at an empty directory (no `git`, no `bash`, no coreutils visible), `conformance`
exits 0 and `supervise` spawns events with zero errors (E11-negative-cases.log, case 6).

- **git version floor = what the image ships: `git version 2.39.5`** (E0-image-selftest.log, E12-runtime-deps.log). Keep it installed for packages that use the git SDK.
- Git identity (`user.email`/`user.name`) was **never demanded** in any run (E4-git-ladder.log, E11-negative-cases.log).

## Q4 — minimal runnable package

**Verdict: two files.** `departments/hello/main.lua` + `raisers/tick.lua` is the smallest tree that
passes `conformance` (exit 0, `PASS graph-scan loaded 1 departments, 1 raisers, 1 queues`) AND
processes events under `supervise` (4 `framework spawned` in 5 s, ≈1/s)
(E5-minimal-tree.log). Full file contents and the per-subcommand optionality matrix are in the
[Minimal package](#minimal-package) section.

Key divergences (E5-minimal-tree.log):

- **Raiser-less package**: `conformance` still passes — `PASS schema-validation schema validation passed with 1 warnings`; under `supervise` the warning text is `queue 'tick' is consumed by department 'hello' but has no producer`, the consumer starts (`consumer started dept=hello`) and then **stays idle forever** (0 spawns). The conformance-passes-but-supervise-idles divergence is real.
- **Raiser-only package** (no departments): `conformance` exit **1** — `FAIL department-non-empty host graph contains no departments` (an absent `departments/` dir itself is tolerated by the layout check: `PASS project-layout host departments directory absent`).
- Baseline: `examples/minimal-package` passes with exit 0, `PASS graph-scan loaded 2 departments, 1 raisers, 2 queues` (E2-baseline-conformance.log).

## Q5 — `FKST_RUNTIME_ROOT`: never validated by the engine; the RUNNER must pre-validate

**Verdict: `conformance` ignores it entirely; `supervise` self-creates the log tree but
half-alive-panics when the var is unset and silently drops logs when it is non-writable. The
engine never fail-closes on this variable.** (E6-runtime-root.log)

- `conformance` **never touches it**: a fresh dir stays empty after a run; with the var **unset** it still exits 0 with `PASS runtime-layout FKST_RUNTIME_ROOT not set; runtime scratch unused by conformance`; a **read-only** (`dr-xr-xr-x` root-owned) or **nonexistent** path is also accepted — `PASS runtime-layout runtime root accepted: /opt/ro-runtime` / `PASS runtime-layout runtime root accepted: /tmp/no-such-dir/runtime`. Conformance does NOT validate writability (E6-runtime-root.log).
- **`supervise` creates `logs/framework-child/` itself, including all missing parents** (`mkdir -p` semantics): pointing it at the nonexistent `/tmp/no-such-dir/runtime` produced the full `logs/framework-child/` tree with child logs inside (E6-runtime-root.log). No pre-created subdirs are needed; only the euid must be able to create/write the path.
- `supervise` with the var **unset**: the process **starts and keeps running** (exit 124 under the timeout harness) but every consumer thread panics — verbatim:

  ```
  thread 'main' (236) panicked at crates/fkst-framework/src/supervise/consumer.rs:59:14:
  runtime layout should be valid: FKST_RUNTIME_ROOT must be set
  note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
  ```

  followed by repeating `WARN fkst_framework::supervise::delivery_router: reliable wake receiver not registered dept=producer`. **Half-alive zombie mode — no error exit code** (E6-runtime-root.log, E11-negative-cases.log case 7b).
- `supervise` with a **non-writable** root (tested both root-owned `0555` and owner-`0555`): the engine **keeps running and processing**, but child logs are silently dropped — every dispatch reports `log_path=None` (E6-runtime-root.log incl. addendum, E11-negative-cases.log case 7d).

## Q6 — `FKST_DURABLE_ROOT`: required to start for any default (reliable) graph

**Verdict: required iff the graph has ≥1 reliable subscription — and consumes are reliable by
default, so it is effectively required for any normal package. Fail-fast, exit 2. No default
location.** This settles the doc-vs-code conflict: both statements are true and compose — the
condition is the subscription mode.

- Missing, with a reliable graph (`examples/minimal-package`) — verbatim (E3-hostfacts-ladder.log, E7-durable-root.log):

  ```
  [framework] startup error: FKST_DURABLE_ROOT must be set
  ERROR fkst_framework::supervise: durable layout required for reliable subscriptions error=FKST_DURABLE_ROOT must be set
  exit=2
  ```

- A package whose every `consumes` entry is listed in `M.spec.ephemeral` starts **without** it and processes events: `consumer started dept=hello reliable_queues=[] ephemeral_queues=["tick"]`, 3 spawns (E7-durable-root.log).
- With it set, the engine writes exactly `$FKST_DURABLE_ROOT/delivery.redb` — observed `-rw-r--r-- 1 fkst fkst 3686400 ... delivery.redb` after seconds of uptime (E7-durable-root.log). Env-only; no default path.
- Delivery/lease state persists in that redb, so a re-run against the *same* durable root resumes prior lease state — the runner must use a **fresh dir per session attempt** (idempotency / redo-from-scratch); not persisting it across pods is compatible with the canon "accepted loss" model.

## Q7 — stop behavior: ~60 ms clean stop for direct `supervise`; the wrapper orphans its child

**Verdict: SIGTERM to the process group of a directly spawned `fkst-framework supervise` stops
everything in ≈58 ms with zero live survivors; PID-only TERM is equally clean. The
`fkst-supervisor` wrapper, by contrast, abandons its running framework child on TERM — the runner
must never use it.** Spawned via `setsid` (PGID == PID confirmed from `/proc/<pid>/stat`)
(E10-stop.log).

| Action on direct `fkst-framework supervise` | Result | Settle (0.05 s poll) |
|---|---|---|
| `kill -TERM -- -$PGID` (group) | clean exit, **0 live survivors** | **58 ms** |
| `kill -TERM $PID` (PID-only) | clean exit, **0 live survivors** | **60 ms** |

Verbatim evidence (E10-stop.log; `/proc`-based ps excluding zombies — columns pid pgid ppid state comm):

```
    25     25     15 R (fkst-framework)      <- before group TERM
+ kill -TERM -- -25
settle: 58 ms (poll 0.05s, live-non-zombie)
OK: no live fkst-framework/fkst-supervisor survivors

    74     74     15 R (fkst-framework)      <- before PID-only TERM
+ kill -TERM 74
settle: 60 ms (poll 0.05s, live-non-zombie)
OK: no live survivors (PID-only TERM sufficient for direct supervise)
```

Contrast — `fkst-supervisor` wrapper (E10-stop.log):

```
   121    121     15 S (fkst-supervisor)     <- wrapper
   123    123    121 R (fkst-framework)      <- child: NOTE its own pgid (123 != 121)
+ kill -TERM 121
WARN fkst_supervisor: supervisor exiting without signaling event runtime pid=123 signal="terminate"
   123    123      1 R (fkst-framework)      <- orphan: reparented to PID 1, still running
+ kill -TERM -- -121                          <- group-kill of the wrapper's group: exit=1
   123    123      1 R (fkst-framework)      <- orphan STILL alive
+ kill -TERM 123                              <- only a direct TERM reaches it
orphan settle after direct TERM: 57 ms
```

Because the wrapper starts its child with its own process group (`spawner.rs:97
process_group(0)` in the pinned source), **even a group-kill of the wrapper's group cannot reach
the child** — the session-runner must spawn `fkst-framework supervise` directly and never the
wrapper. Per-event `run` children also get their own process groups but are sub-second one-shots;
none survived in any trial (E10-stop.log).

**Reaping caveat:** in a container whose PID 1 does not reap (the probe container's PID 1 was
`sleep`), terminated orphans persist as **zombies** and pollute naive PID-table liveness checks.
The first E10 attempt was invalidated by exactly this and is kept as
`E10-stop-INVALID-zombie-artifact.log`. The runner stays the parent and `waitpid`s, so this is
only a hazard for PID-table-scanning health checks — match on state != `Z`.

## Q8 — flags and env substitution

**Verdicts** (E8-flags.log unless noted):

- **There is no `--help`**: `fkst-framework --help` → exit **2**, `[framework] startup error: unknown subcommand: --help`. Exit 2 is the generic SDK/IO/arg-parse error code; introspection must use the `config` subcommand (which itself requires a project root AND a package root — even an empty directory pair).
- `--framework-bin` is **mandatory** for `supervise`: exit **2**, `[framework] startup error: missing --framework-bin`. It points at **`/usr/local/bin/fkst-framework` itself** — the same binary, re-invoked per event; verbatim `CMD=` header in every child log (E9-supervise-block.log):

  ```
  CMD=/usr/local/bin/fkst-framework run /tmp/tmp.z9pXTuq7U0/departments/consumer/main.lua --project-root /tmp/tmp.z9pXTuq7U0 --package-root /tmp/tmp.z9pXTuq7U0 --owner-namespace tmp.z9pXTuq7U0 --event <json>
  ```

- `--project-root` is **mandatory** for every subcommand: exit **2**, `[framework] startup error: missing --project-root` (also E1-config-registry.log for `config`).
- A package root is **mandatory** for every subcommand (even `config`): exit **2**, `[framework] startup error: FKST_PACKAGE_ROOTS, FKST_PACKAGE_ROOT, or --package-root is required` (E1-config-registry.log, E11-negative-cases.log case 3a).
- **`FKST_PACKAGE_ROOT` env DOES substitute for `--package-root`** (the "likely NO" prior is refuted) — both `conformance` and `supervise` ran clean with env-only package-root wiring; the plural `FKST_PACKAGE_ROOTS` also works (E8-flags.log). `--project-root` and `--framework-bin` remain flag-only.
- `fkst-supervisor` parses no args: project root = **cwd**, framework bin = `FKST_FRAMEWORK_BIN` env or the binary next to its own exe (E8-flags.log; informational only — see Q7 for why the runner never uses it).

## Q9 — liveness / ready markers

**Verdict: primary signal = PID liveness of the `supervise` process, plus grep-able ready markers
on its stderr** (`tracing`, INFO level, ANSI-colored) (E9-supervise-block.log):

```
INFO fkst_framework::supervise: event runtime running handles=3
INFO fkst_framework::supervise::consumer: consumer started dept=consumer reliable_queues=["example_event"] ephemeral_queues=[]
INFO fkst_framework::supervise::consumer: consumer started dept=producer reliable_queues=["tick"] ephemeral_queues=[]
INFO fkst_framework::supervise::source_runner: cron raiser starting raiser=tick interval_s=1
```

- `event runtime running handles=<n>` — runtime fully wired (one line, after graph scan/validation): **the ready marker**.
- `consumer started dept=<name> reliable_queues=[...] ephemeral_queues=[...]` — per department.
- `cron raiser starting raiser=<name> interval_s=<n>` — per raiser.
- Per-dispatch: `framework spawned pid=<n> lua=<path>` / `framework ok dept=... log_path=Some(...)` / `delivery acked ...` (E3-hostfacts-ladder.log, E9-supervise-block.log).

**Child log files**: `<FKST_RUNTIME_ROOT>/logs/framework-child/<dept>-<unix-secs>-<nanos>-<seq>.log`
(e.g. `consumer-1781149719-553798510-2.log`); header lines `CMD=`, `LUA=`, `HOST_ROOT=`,
`PACKAGE_ROOTS=`, `OWNER_NAMESPACE=`, then the Lua run's output (E9-supervise-block.log). These
appear **only after the first dispatched event** — a raiser-less (idle) session produces ready
markers but no child logs (E5-minimal-tree.log).

---

## Error classification table

Every negative case, with the verbatim key stderr line and its exit code. Three probed cases
turned out to be **non-errors** (rows 1, 2, 6) — they are recorded as such and must NOT be mapped
to API errors. Phase: pre-flight = deterministic before/at `conformance` (runner maps to `400`);
runtime = only observable under `supervise` (runner maps to `500` or must pre-validate).

| # | Case | Command | Verbatim stderr (key line) | Exit | Phase | API mapping | Evidence |
|---|---|---|---|---|---|---|---|
| 1 | empty/missing `fkst.env` | `conformance` / `supervise` | — (no error; all checks PASS, supervise runs) | 0 / runs | n/a | **not an error** | E3-hostfacts-ladder.log, E5-minimal-tree.log |
| 2 | non-git package dir | `conformance` / `supervise` | — (no error; zero git output) | 0 / runs | n/a | **not an error** | E4-git-ladder.log |
| 3a | no package root (flag + env absent) | any subcommand | `[framework] startup error: FKST_PACKAGE_ROOTS, FKST_PACKAGE_ROOT, or --package-root is required` | 2 | pre-flight | 400 | E11-negative-cases.log, E1-config-registry.log |
| 3b | package root = empty dir | `conformance` | `FAIL department-non-empty host graph contains no departments` | 1 | pre-flight | 400 | E11-negative-cases.log |
| 3c | package root nonexistent | `conformance` | `[framework] startup error: canonicalize --project-root /tmp/definitely-missing-xyz: No such file or directory (os error 2)` | 2 | pre-flight | 400 | E11-negative-cases.log |
| 3d | package root = empty dir | `supervise` | — **starts and idles** (`event runtime running handles=0`; no fail-closed) | runs | runtime | none — must be blocked by runner pre-flight | E11-negative-cases.log |
| 4 | `main.lua` without `M.spec` | `conformance` | ``FAIL graph-scan graph scan failed: department `broken` missing `M.spec`: error converting Lua nil to table`` | 1 | pre-flight | 400 | E11-negative-cases.log |
| 4' | same | `supervise` | ``[framework] startup error: department `broken` missing `M.spec`: error converting Lua nil to table`` | 2 | startup (fail-fast) | 400/500 | E11-negative-cases.log |
| 5 | `main.lua` without global `pipeline` | `conformance` | — **passes** (NOT caught pre-flight) | 0 | — | — | E11-negative-cases.log |
| 5' | same | `supervise` | repeating ``WARN ... framework failed dept=nopipe exit=1 ... stderr=[framework] pipeline failed: lua file did not define global `pipeline` function: error converting Lua nil to function`` | runs (per-event exit 1; reliable retry re-dispatches) | runtime | 500 | E11-negative-cases.log |
| 6 | `git` + `bash` stripped from `PATH` | `conformance` / `supervise` | — (no error; engine has no shell-outs in this flow) | 0 / runs | n/a | **not an error** | E11-negative-cases.log |
| 7a | `FKST_RUNTIME_ROOT` unset | `conformance` | `PASS runtime-layout FKST_RUNTIME_ROOT not set; runtime scratch unused by conformance` | 0 | — | — | E6-runtime-root.log, E11-negative-cases.log |
| 7b | `FKST_RUNTIME_ROOT` unset | `supervise` | `thread 'main' (236) panicked at crates/fkst-framework/src/supervise/consumer.rs:59:14:` / `runtime layout should be valid: FKST_RUNTIME_ROOT must be set` — **process keeps running** | runs (half-alive) | runtime | 500 + runner must pre-validate | E6-runtime-root.log, E11-negative-cases.log |
| 7c | `FKST_RUNTIME_ROOT` non-writable | `supervise` | — runs; every dispatch `log_path=None` (child logs silently dropped) | runs | runtime | none surfaced — runner must pre-validate writability | E6-runtime-root.log, E11-negative-cases.log |
| 8 | missing `FKST_DURABLE_ROOT` (reliable graph) | `supervise` | `[framework] startup error: FKST_DURABLE_ROOT must be set` + `ERROR fkst_framework::supervise: durable layout required for reliable subscriptions error=FKST_DURABLE_ROOT must be set` | 2 | startup (fail-fast) | 500 (config) | E3-hostfacts-ladder.log, E7-durable-root.log |

Exit-code legend (observed): `0` ok, `1` lua/pipeline/conformance-check failure, `2`
SDK/IO/arg-parse error, `124` = the spike's `timeout` harness (process still running = healthy
blocking run).

---

## Minimal package

The exact smallest tree that passes `conformance` AND processes events under `supervise`
(E5-minimal-tree.log):

```
$PKG/
├── departments/hello/main.lua
└── raisers/tick.lua
```

Materialize it exactly like this (verified `M.spec` shape — **no `name` field**; identity is the
directory name):

```bash
PKG="$(mktemp -d)"
mkdir -p "$PKG/departments/hello" "$PKG/raisers"

cat > "$PKG/departments/hello/main.lua" <<'LUA'
local M = {}
M.spec = {
  consumes = { "tick" },
  stall_window = "30s",
}
function pipeline(event)
  log.info("hello received event on queue: " .. tostring(event.queue))
end
return M
LUA

cat > "$PKG/raisers/tick.lua" <<'LUA'
return {
  type = "cron",
  interval = "1s",
  produces = "tick",
}
LUA
```

Result: `conformance` exit 0 (`PASS graph-scan loaded 1 departments, 1 raisers, 1 queues`);
`supervise` spawns ≈1 framework child per second (E5-minimal-tree.log).

Per-subcommand optionality (E5-minimal-tree.log, E7-durable-root.log, E11-negative-cases.log):

| Element | `conformance` | `supervise` |
|---|---|---|
| `departments/<name>/main.lua` with `M.spec` | **required** — `FAIL department-non-empty` if none, exit 1 | **NOT enforced** — starts and idles on an empty or raiser-only package (exit only by signal). Pre-flight is mandatory in the runner. |
| `raisers/` | optional — `loaded 1 departments, 0 raisers, 1 queues`, `PASS schema-validation schema validation passed with 1 warnings` (warning: `queue 'tick' is consumed by department 'hello' but has no producer`) | optional — starts, emits `consumer started`, then **stays idle** (0 spawns). Conformance-pass-with-warning vs supervise-idle divergence. |
| `M.spec.stall_window` | optional — default 30 s applied (`stall_window_ms=30000`) | optional |
| `core.lua` | absent in all passing runs → **optional** | optional |
| `composed.deps` | absent in all passing runs → **optional** | optional |
| `M.spec.ephemeral = { "<queue>" }` | optional | controls reliable-vs-ephemeral delivery; drives the `FKST_DURABLE_ROOT` start requirement (Q6) |

Raiser-only package (departments absent): `conformance` exit **1**, `FAIL department-non-empty
host graph contains no departments`; the layout check itself tolerates the absent dir (`PASS
project-layout host departments directory absent`) (E5-minimal-tree.log).

---

## Reproduction recipe

The complete experiment ladder (E2–E11) as one self-contained script, verbatim. Run it inside the
engine image with `examples/minimal-package` from the pinned `fkst-substrate` checkout mounted
read-only (the `docker run` invocation is in the header comment). It emits normalized
`VERDICT <id> ...` lines for cross-run diffing — two fresh-container passes produced byte-identical
verdict sets (REPRO.log).

```bash
#!/usr/bin/env bash
# spike17 consolidated experiment script (E2-E11) for issue #17.
# Run inside the engine image with /opt/minimal-package mounted read-only:
#   docker run --rm \
#     -v /tmp/fkst-substrate-ro/examples/minimal-package:/opt/minimal-package:ro \
#     -v /tmp/spike17/spike.sh:/spike.sh:ro \
#     --entrypoint bash fkst-hosted-api:engine-dev /spike.sh
# Emits normalized "VERDICT <id> ..." lines (temp paths scrubbed) for cross-run diffing.
set -x
FW=/usr/local/bin/fkst-framework
SUPB=/usr/local/bin/fkst-supervisor
scrub() { sed -e 's|/tmp/tmp\.[A-Za-z0-9]*|<TMP>|g' -e 's|([0-9]*)|(N)|g'; }
verdict() { echo "VERDICT $*"; }
psx() { for s in /proc/[0-9]*/stat; do awk '{printf "%6d %6d %6d %s %s\n", $1, $5, $4, $3, $2}' "$s" 2>/dev/null; done; }
livefkst() { psx | awk '$4 != "Z" && $5 ~ /fkst/'; }
mkpkg() { local P; P=$(mktemp -d); cp -R /opt/minimal-package/. "$P/"; echo "$P"; }

# ===== E2: baseline conformance with 2-key fkst.env =====
PKG=$(mkpkg); RT=$(mktemp -d); DR=$(mktemp -d)
printf 'FKST_CANDIDATE_PREFIX=candidate/\nFKST_CANDIDATE_FROM_SEP=::\n' > "$PKG/fkst.env"
OUT=$(env FKST_RUNTIME_ROOT="$RT" "$FW" conformance --project-root "$PKG" --package-root "$PKG" 2>&1); RC=$?
verdict E2-conformance "exit=$RC loaded=$(echo "$OUT" | grep -o 'loaded 2 departments, 1 raisers, 2 queues')"
rm -rf "$PKG"

# ===== E3: HostFacts ladder =====
PKG=$(mkpkg)  # NO fkst.env at all
RC=$(env FKST_RUNTIME_ROOT="$RT" "$FW" conformance --project-root "$PKG" --package-root "$PKG" >/dev/null 2>&1; echo $?)
verdict E3-conformance-no-fkstenv "exit=$RC"
ERR=$(timeout 3 env -u FKST_DURABLE_ROOT FKST_RUNTIME_ROOT="$RT" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" 2>&1 >/dev/null | grep 'startup error' | scrub); RC=$?
verdict E3-supervise-no-durable "exit-recorded-below err=[$ERR]"
timeout 3 env -u FKST_DURABLE_ROOT FKST_RUNTIME_ROOT="$RT" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" >/dev/null 2>&1
verdict E3-supervise-no-durable-exit "exit=$?"
RC=$(timeout 3 env FKST_RUNTIME_ROOT="$RT" FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" >/dev/null 2>&1; echo $?)
verdict E3-supervise-durable-no-fkstenv "exit=$RC (124=started+blocked)"
rm -rf "$PKG"

# ===== E4: git not required =====
PKG=$(mkpkg)
git -C "$PKG" rev-parse --is-inside-work-tree >/dev/null 2>&1; verdict E4-not-a-git-repo "git-check-exit=$?"
RC=$(env FKST_RUNTIME_ROOT="$RT" "$FW" conformance --project-root "$PKG" --package-root "$PKG" >/dev/null 2>&1; echo $?)
verdict E4-conformance-non-git "exit=$RC"
RC=$(timeout 3 env FKST_RUNTIME_ROOT="$RT" FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" >/dev/null 2>&1; echo $?)
verdict E4-supervise-non-git "exit=$RC (124=started+blocked)"
rm -rf "$PKG"

# ===== E5: minimal hand-written package + ablations =====
PKG=$(mktemp -d); mkdir -p "$PKG/departments/hello" "$PKG/raisers"
cat > "$PKG/departments/hello/main.lua" <<'LUA'
local M = {}
M.spec = {
  consumes = { "tick" },
  stall_window = "30s",
}
function pipeline(event)
  log.info("hello received event on queue: " .. tostring(event.queue))
end
return M
LUA
printf 'return {\n  type = "cron",\n  interval = "1s",\n  produces = "tick",\n}\n' > "$PKG/raisers/tick.lua"
RC=$(env FKST_RUNTIME_ROOT="$RT" "$FW" conformance --project-root "$PKG" --package-root "$PKG" >/dev/null 2>&1; echo $?)
verdict E5-V1-minimal-conformance "exit=$RC"
timeout 4 env FKST_RUNTIME_ROOT="$RT" FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" > /tmp/v1.log 2>&1
verdict E5-V1-minimal-supervise "exit=$? spawned=$(grep -c 'framework spawned' /tmp/v1.log | awk '{print ($1>0)?"yes":"no"}')"
sed -i '/stall_window/d' "$PKG/departments/hello/main.lua"
RC=$(env FKST_RUNTIME_ROOT="$RT" "$FW" conformance --project-root "$PKG" --package-root "$PKG" >/dev/null 2>&1; echo $?)
verdict E5-V1b-no-stallwindow-conformance "exit=$RC"
rm -rf "$PKG/raisers"
OUT=$(env FKST_RUNTIME_ROOT="$RT" "$FW" conformance --project-root "$PKG" --package-root "$PKG" 2>&1); RC=$?
verdict E5-V2-no-raisers-conformance "exit=$RC loaded=$(echo "$OUT" | grep -o 'loaded 1 departments, 0 raisers, 1 queues')"
timeout 4 env FKST_RUNTIME_ROOT="$RT" FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" > /tmp/v2.log 2>&1
verdict E5-V2-no-raisers-supervise "exit=$? spawned-count=$(grep -c 'framework spawned' /tmp/v2.log) (idle expected)"
PKG3=$(mktemp -d); mkdir -p "$PKG3/raisers"
printf 'return {\n  type = "cron",\n  interval = "1s",\n  produces = "tick",\n}\n' > "$PKG3/raisers/tick.lua"
OUT=$(env FKST_RUNTIME_ROOT="$RT" "$FW" conformance --project-root "$PKG3" --package-root "$PKG3" 2>&1); RC=$?
verdict E5-V3-raiser-only-conformance "exit=$RC fail=$(echo "$OUT" | grep -o 'FAIL department-non-empty host graph contains no departments')"
rm -rf "$PKG" "$PKG3" /tmp/v1.log /tmp/v2.log

# ===== E6: runtime root =====
PKG=$(mkpkg); RT2=$(mktemp -d)
env FKST_RUNTIME_ROOT="$RT2" "$FW" conformance --project-root "$PKG" --package-root "$PKG" >/dev/null 2>&1
verdict E6-conformance-touches-rt "contents-after=[$(ls -A "$RT2" | tr '\n' ' ')]"
timeout 3 env FKST_RUNTIME_ROOT="$RT2" FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" >/dev/null 2>&1
verdict E6-supervise-creates-logdir "exists=$( [ -d "$RT2/logs/framework-child" ] && echo yes || echo no )"
OUT=$(env -u FKST_RUNTIME_ROOT "$FW" conformance --project-root "$PKG" --package-root "$PKG" 2>&1); RC=$?
verdict E6-conformance-rt-unset "exit=$RC line=[$(echo "$OUT" | grep -o 'FKST_RUNTIME_ROOT not set; runtime scratch unused by conformance')]"
timeout 3 env -u FKST_RUNTIME_ROOT FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" > /tmp/e6b.log 2>&1
verdict E6-supervise-rt-unset "exit=$? panic=$(grep -c 'runtime layout should be valid: FKST_RUNTIME_ROOT must be set' /tmp/e6b.log | awk '{print ($1>0)?"yes":"no"}') (124=half-alive zombie mode)"
mkdir -p /tmp/ro-rt 2>/dev/null; chmod 555 /tmp/ro-rt 2>/dev/null
RC=$(env FKST_RUNTIME_ROOT=/tmp/ro-rt "$FW" conformance --project-root "$PKG" --package-root "$PKG" >/dev/null 2>&1; echo $?)
verdict E6-conformance-rt-readonly "exit=$RC (writability NOT pre-flighted)"
timeout 3 env FKST_RUNTIME_ROOT=/tmp/ro-rt FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" > /tmp/e6c.log 2>&1
verdict E6-supervise-rt-readonly "exit=$? logpath-none=$(grep -c 'log_path=None' /tmp/e6c.log | awk '{print ($1>0)?"yes":"no"}') (runs; logs silently dropped)"
RC=$(env FKST_RUNTIME_ROOT=/tmp/nonexistent-rt-path/runtime "$FW" conformance --project-root "$PKG" --package-root "$PKG" >/dev/null 2>&1; echo $?)
timeout 3 env FKST_RUNTIME_ROOT=/tmp/nonexistent-rt-path/runtime FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" >/dev/null 2>&1
verdict E6-rt-nonexistent "conformance-exit=$RC supervise-created=$( [ -d /tmp/nonexistent-rt-path/runtime/logs/framework-child ] && echo yes || echo no )"
rm -rf "$PKG" "$RT2" /tmp/e6b.log /tmp/e6c.log /tmp/nonexistent-rt-path; chmod 755 /tmp/ro-rt 2>/dev/null; rm -rf /tmp/ro-rt

# ===== E7: durable root =====
PKG=$(mkpkg)
ERR=$(timeout 3 env -u FKST_DURABLE_ROOT FKST_RUNTIME_ROOT="$RT" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" 2>&1 >/dev/null | grep 'startup error' | scrub)
verdict E7-reliable-no-durable "err=[$ERR]"
DR2=$(mktemp -d)
timeout 3 env FKST_RUNTIME_ROOT="$RT" FKST_DURABLE_ROOT="$DR2" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" >/dev/null 2>&1
verdict E7-redb-location "exit=$? redb=$( [ -f "$DR2/delivery.redb" ] && echo 'FKST_DURABLE_ROOT/delivery.redb' || echo MISSING )"
PKG2=$(mktemp -d); mkdir -p "$PKG2/departments/hello" "$PKG2/raisers"
cat > "$PKG2/departments/hello/main.lua" <<'LUA'
local M = {}
M.spec = {
  consumes = { "tick" },
  ephemeral = { "tick" },
  stall_window = "30s",
}
function pipeline(event)
  log.info("hello received event on queue: " .. tostring(event.queue))
end
return M
LUA
printf 'return {\n  type = "cron",\n  interval = "1s",\n  produces = "tick",\n}\n' > "$PKG2/raisers/tick.lua"
timeout 4 env -u FKST_DURABLE_ROOT FKST_RUNTIME_ROOT="$RT" "$FW" supervise --project-root "$PKG2" --package-root "$PKG2" --framework-bin "$FW" > /tmp/e7c.log 2>&1
verdict E7-pure-ephemeral-no-durable "exit=$? spawned=$(grep -c 'framework spawned' /tmp/e7c.log | awk '{print ($1>0)?"yes":"no"}') (124=starts fine)"
rm -rf "$PKG" "$PKG2" "$DR2" /tmp/e7c.log

# ===== E8: flags =====
PKG=$(mkpkg)
ERR=$("$FW" --help 2>&1); RC=$?
verdict E8-no-help "exit=$RC err=[$ERR]"
ERR=$(env FKST_RUNTIME_ROOT="$RT" FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$PKG" --package-root "$PKG" 2>&1); RC=$?
verdict E8-missing-framework-bin "exit=$RC err=[$ERR]"
ERR=$(env FKST_RUNTIME_ROOT="$RT" FKST_DURABLE_ROOT="$DR" "$FW" supervise --package-root "$PKG" --framework-bin "$FW" 2>&1); RC=$?
verdict E8-missing-project-root "exit=$RC err=[$ERR]"
RC=$(env FKST_RUNTIME_ROOT="$RT" FKST_PACKAGE_ROOT="$PKG" "$FW" conformance --project-root "$PKG" >/dev/null 2>&1; echo $?)
verdict E8-env-FKST_PACKAGE_ROOT-substitutes "conformance-exit=$RC"
RC=$(env FKST_RUNTIME_ROOT="$RT" FKST_PACKAGE_ROOTS="$PKG" "$FW" conformance --project-root "$PKG" >/dev/null 2>&1; echo $?)
verdict E8-env-FKST_PACKAGE_ROOTS-substitutes "conformance-exit=$RC"
ERR=$(env -u FKST_PACKAGE_ROOT -u FKST_PACKAGE_ROOTS FKST_RUNTIME_ROOT="$RT" "$FW" conformance --project-root "$PKG" 2>&1); RC=$?
verdict E8-no-package-root-anywhere "exit=$RC err=[$(echo "$ERR" | scrub)]"

# ===== E9/E10: spawn, ready markers, stop =====
RT3=$(mktemp -d)
setsid env FKST_RUNTIME_ROOT="$RT3" FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" > /tmp/sup.log 2>&1 < /dev/null &
P=$!
sleep 3
verdict E9-spawned "pid-alive=$( kill -0 $P 2>/dev/null && echo yes || echo no ) pgid-equals-pid=$( [ "$(awk '{print $5}' /proc/$P/stat)" = "$P" ] && echo yes || echo no )"
verdict E9-ready-markers "runtime-running=$(grep -c 'event runtime running' /tmp/sup.log | awk '{print ($1>0)?"yes":"no"}') consumer-started=$(grep -c 'consumer started' /tmp/sup.log | awk '{print ($1>0)?"yes":"no"}')"
LOGPAT=$(ls "$RT3/logs/framework-child/" 2>/dev/null | head -1 | sed 's/[0-9]\{6,\}/<N>/g')
verdict E9-child-log-pattern "[$LOGPAT]"
T0=$(date +%s%N)
kill -TERM -- -"$P"
i=0; while [ -n "$(livefkst)" ] && [ $i -lt 300 ]; do sleep 0.05; i=$((i+1)); done
T1=$(date +%s%N); MS=$(( (T1-T0)/1000000 ))
verdict E10a-group-term "survivors=$(livefkst | wc -l | tr -d ' ') settle-under-2s=$( [ $MS -lt 2000 ] && echo yes || echo no )"
setsid env FKST_RUNTIME_ROOT="$RT3" FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" > /tmp/sup2.log 2>&1 < /dev/null &
P2=$!
sleep 3
T0=$(date +%s%N)
kill -TERM "$P2"
i=0; while [ -n "$(livefkst)" ] && [ $i -lt 300 ]; do sleep 0.05; i=$((i+1)); done
T1=$(date +%s%N); MS=$(( (T1-T0)/1000000 ))
verdict E10b-pid-only-term "survivors=$(livefkst | wc -l | tr -d ' ') settle-under-2s=$( [ $MS -lt 2000 ] && echo yes || echo no )"
cd "$PKG"
setsid env FKST_RUNTIME_ROOT="$RT3" FKST_DURABLE_ROOT="$DR" FKST_PACKAGE_ROOT="$PKG" "$SUPB" > /tmp/sup3.log 2>&1 < /dev/null &
P3=$!
sleep 3
kill -TERM "$P3"
sleep 2
ORPH=$(livefkst | awk '$5 ~ /fkst-framework/ {print $1}' | head -1)
verdict E10c-supervisor-wrapper-orphans "orphan-running=$( [ -n "$ORPH" ] && echo yes || echo no ) warn=$(grep -c 'exiting without signaling event runtime' /tmp/sup3.log | awk '{print ($1>0)?"yes":"no"}')"
[ -n "$ORPH" ] && kill -TERM "$ORPH"
sleep 1
verdict E10-final-clean "live=$(livefkst | wc -l | tr -d ' ')"
cd /
rm -rf "$PKG" "$RT3" /tmp/sup.log /tmp/sup2.log /tmp/sup3.log

# ===== E11: negative cases =====
PKG=$(mktemp -d); mkdir -p "$PKG/departments/broken"
printf 'local M = {}\nfunction pipeline(event)\n  log.info("x")\nend\nreturn M\n' > "$PKG/departments/broken/main.lua"
OUT=$(env FKST_RUNTIME_ROOT="$RT" "$FW" conformance --project-root "$PKG" --package-root "$PKG" 2>&1); RC=$?
verdict E11-missing-Mspec-conformance "exit=$RC fail=[$(echo "$OUT" | grep -o 'department `broken` missing `M.spec`: error converting Lua nil to table' | head -1)]"
ERR=$(timeout 3 env FKST_RUNTIME_ROOT="$RT" FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" 2>&1 >/dev/null | grep 'startup error' | scrub);
timeout 3 env FKST_RUNTIME_ROOT="$RT" FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" >/dev/null 2>&1
verdict E11-missing-Mspec-supervise "exit=$? err=[$ERR]"
rm -rf "$PKG"
PKG=$(mktemp -d); mkdir -p "$PKG/departments/nopipe" "$PKG/raisers"
printf 'local M = {}\nM.spec = {\n  consumes = { "tick" },\n  stall_window = "30s",\n}\nreturn M\n' > "$PKG/departments/nopipe/main.lua"
printf 'return {\n  type = "cron",\n  interval = "1s",\n  produces = "tick",\n}\n' > "$PKG/raisers/tick.lua"
RC=$(env FKST_RUNTIME_ROOT="$RT" "$FW" conformance --project-root "$PKG" --package-root "$PKG" >/dev/null 2>&1; echo $?)
verdict E11-missing-pipeline-conformance "exit=$RC (NOT caught pre-flight)"
timeout 4 env FKST_RUNTIME_ROOT="$RT" FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" > /tmp/np.log 2>&1
verdict E11-missing-pipeline-supervise "exit=$? runtime-failure=$(grep -c 'did not define global `pipeline` function' /tmp/np.log | awk '{print ($1>0)?"yes":"no"}')"
rm -rf "$PKG" /tmp/np.log
PKG=$(mkpkg); MIN=$(mktemp -d)
RC=$(env PATH="$MIN" FKST_RUNTIME_ROOT="$RT" "$FW" conformance --project-root "$PKG" --package-root "$PKG" >/dev/null 2>&1; echo $?)
verdict E11-no-git-bash-on-PATH-conformance "exit=$RC"
timeout 3 env PATH="$MIN" FKST_RUNTIME_ROOT="$RT" FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$PKG" --package-root "$PKG" --framework-bin "$FW" > /tmp/p6.log 2>&1
verdict E11-no-git-bash-on-PATH-supervise "exit=$? spawned=$(grep -c 'framework spawned' /tmp/p6.log | awk '{print ($1>0)?"yes":"no"}')"
EMPTY=$(mktemp -d)
RC=$(timeout 3 env FKST_RUNTIME_ROOT="$RT" FKST_DURABLE_ROOT="$DR" "$FW" supervise --project-root "$EMPTY" --package-root "$EMPTY" --framework-bin "$FW" >/dev/null 2>&1; echo $?)
verdict E11-supervise-empty-package "exit=$RC (124=starts and idles; NOT fail-closed)"
ERR=$(env FKST_RUNTIME_ROOT="$RT" "$FW" conformance --project-root /tmp/definitely-missing-xyz --package-root /tmp/definitely-missing-xyz 2>&1); RC=$?
verdict E11-nonexistent-root "exit=$RC err=[$ERR]"
rm -rf "$PKG" "$MIN" "$EMPTY" /tmp/p6.log "$RT" "$DR"
echo "SPIKE-COMPLETE"
```

---

## Runner contract summary

The contract the session-runner (#18) and engine-Dockerfile (#21) issues consume.

| Aspect | Contract |
|---|---|
| Mandatory env — to start (engine-enforced) | **NONE** for a fully-ephemeral graph (E7-durable-root.log). **`FKST_DURABLE_ROOT=<fresh dir>`** for any graph with ≥1 reliable subscription — and consumes are reliable by default, so effectively always; engine fail-fast exit 2 if missing; redb lands at `$FKST_DURABLE_ROOT/delivery.redb` (E3-hostfacts-ladder.log, E7-durable-root.log). |
| Mandatory env — runner-enforced | **`FKST_RUNTIME_ROOT=<fresh writable dir>`** must be set AND pre-validated (exists/creatable + writable) by the RUNNER, because `supervise` half-alive-panics when it is unset and silently drops child logs (`log_path=None`) when it is non-writable — the engine never fail-closes on it (E6-runtime-root.log, E11-negative-cases.log). The engine creates `logs/framework-child/` itself (`mkdir -p` semantics). |
| HostFacts | The 2 keys (`FKST_CANDIDATE_PREFIX`, `FKST_CANDIDATE_FROM_SEP`) are only needed by SDK/candidate-git code paths, not to start (E1-config-registry.log, E3-hostfacts-ladder.log, E5-minimal-tree.log). Materialize a 2-key `fkst.env` anyway; all operational keys have defaults. |
| Materialize | **Plain directory** (tempfile-style) — **NO git repo, no `git init`/commit/identity** (E4-git-ladder.log); git ships in the image only for the Lua `sdk_git` API. Layout: `departments/<name>/main.lua` (+ optional `raisers/*.lua`). Fresh `FKST_RUNTIME_ROOT` and `FKST_DURABLE_ROOT` per session attempt — a stale `delivery.redb` replays lease state (E7-durable-root.log). |
| Pre-flight | `fkst-framework conformance --project-root $PKG --package-root $PKG` → exit 0 required. **Mandatory** — `supervise` is not a validator: it idles on an empty package (E11-negative-cases.log case 3d) and misses the missing-`pipeline` case entirely (runtime-only, E11-negative-cases.log case 5'). |
| Spawn | Own session/PGID (`setsid`-equivalent): `fkst-framework supervise --project-root <pkg> --package-root <pkg> --framework-bin $(command -v fkst-framework)`, stdout+stderr captured. `FKST_PACKAGE_ROOT` (or `FKST_PACKAGE_ROOTS`) env substitutes for `--package-root`; `--project-root` and `--framework-bin` are flag-only (E8-flags.log). **NEVER spawn the `fkst-supervisor` wrapper — it orphans its child by design (`process_group(0)`); even a group-kill of the wrapper's group cannot reach the child** (E10-stop.log). |
| Liveness / ready | PID liveness of the supervise process + stderr markers: `event runtime running handles=<n>` (ready), then `consumer started dept=...` per department (E9-supervise-block.log). Child logs `<RT>/logs/framework-child/<dept>-<secs>-<nanos>-<seq>.log` appear **only after the first dispatched event** (E5-minimal-tree.log, E9-supervise-block.log). |
| Stop | SIGTERM to the process **GROUP** (spawn via setsid, then `kill -TERM -- -<PGID>`); observed settle ≈ **58 ms** (PID-only TERM also clean at 60 ms, but group-kill stays the safe recipe) (E10-stop.log). Bound with a 2–5 s grace then SIGKILL the group. Zombie-aware reaping: the runner must `waitpid`; PID-table health checks must exclude state `Z` (E10-stop.log, E10-stop-INVALID-zombie-artifact.log). |
| Runtime deps | Any glibc base (`ldd`: only `libgcc_s`, `libm`, `libc`, `ld-linux`) (E12-runtime-deps.log). `git 2.39.5` only for the Lua git SDK; `bash` NOT needed for the minimal flow (E11-negative-cases.log case 6); Lua statically vendored — no system `lua*`/`liblua*` anywhere (E12-runtime-deps.log); the image has **no `ps`/procps** — probe `/proc` directly (E0-image-selftest.log). |

Session lifecycle as pinned by this spike:

```mermaid
sequenceDiagram
    participant R as session-runner
    participant FS as filesystem
    participant C as fkst-framework conformance
    participant S as fkst-framework supervise (own PGID)

    R->>FS: materialize plain dir (departments/, raisers/, fkst.env) — no git
    R->>FS: create + validate fresh FKST_RUNTIME_ROOT and FKST_DURABLE_ROOT (writable)
    R->>C: conformance --project-root PKG --package-root PKG
    C-->>R: exit 0 (else map FAIL/exit to 400 and stop)
    R->>S: setsid spawn supervise --project-root PKG --package-root PKG --framework-bin fkst-framework
    S-->>R: stderr "event runtime running handles=n" (ready marker)
    S-->>FS: child logs under RT/logs/framework-child/ (after first event)
    R->>S: kill -TERM -- -PGID (stop)
    S-->>R: clean exit ~58 ms, 0 live survivors; runner waitpid-reaps
```

---

## Design-impacting findings (deltas vs canon)

Findings that **change the session-runner / Dockerfile design** relative to canon assumptions:

1. **No-git-repo finding contradicts the canon's "host must be a git repo" assumption.** Neither `conformance` nor `supervise` ever touches git; the runner's planned `git init` + commit + identity step is dropped entirely (E4-git-ladder.log, E11-negative-cases.log case 6).
2. **HostFacts are NOT required at start (lazy SDK resolution) — contradicts fail-closed-at-boot.** `fkst.env` can be absent for both commands; the fail-closed path only triggers inside SDK code that resolves those keys, so the runner's planned 400-mapping for missing HostFacts is unreachable in pre-flight (E1-config-registry.log, E3-hostfacts-ladder.log, E5-minimal-tree.log).
3. **`FKST_DURABLE_ROOT` is required to start for default reliable consumes** — this settles the source conflict: both code comments are true, the condition is the subscription mode (`M.spec.ephemeral`). Effectively required in v1; fail-fast exit 2; redb at `$FKST_DURABLE_ROOT/delivery.redb`; sessions data model should record `durable_dir` alongside `runtime_dir` (E3-hostfacts-ladder.log, E7-durable-root.log).
4. **The `fkst-supervisor` wrapper must not be used by the runner** — SIGTERM to it abandons the running framework child in a separate process group (reparented to PID 1, still running), and group-kill of the wrapper's group cannot reach it (E10-stop.log).
5. **`FKST_PACKAGE_ROOT` env substitution exists** (plus plural `FKST_PACKAGE_ROOTS`) — the runner may prefer pure-env wiring for the package root; `--project-root` and `--framework-bin` remain flag-only (E8-flags.log).
6. **No `--help`** — exit 2 is the generic arg-parse error code (`unknown subcommand: --help`); any tooling that assumed a help surface must use `config` / the pinned source instead (E8-flags.log).
7. **`supervise` has half-alive states, so the RUNNER must pre-validate rather than trust the engine to fail-closed**: unset `FKST_RUNTIME_ROOT` → perpetual panic loop with no error exit; non-writable runtime root → runs with child logs silently dropped; empty package → starts and idles. All structural and environmental validation lives in the runner's pre-flight; `sessions.error` cannot rely on supervise exiting non-zero for these classes (E6-runtime-root.log, E11-negative-cases.log). Related: a missing global `pipeline(event)` passes conformance and only fails per-event at runtime (reliable retry re-dispatches forever) — catch it pre-launch with a per-department `run` smoke, or accept it as a runtime 500 (E11-negative-cases.log case 5').

## Acceptance criteria not fully satisfied

- **`cargo build --release --workspace` was not re-run inside the spike container.** The build's success is evidenced indirectly: the pinned image (`/etc/fkst-engine-sha` = the pinned SHA) ships working binaries built at image creation time, and `fkst-framework --self-test` exits 0 (E0-image-selftest.log). Flagged here as reported by the spike; a from-source rebuild against the pinned SHA belongs to the engine-Dockerfile issue (#21), which pins the same ref.
