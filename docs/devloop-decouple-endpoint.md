# libraries/devloop ambient-M dissolution — endpoint and documented kernel

This documents the honest end state of the `libraries/devloop` ambient-`M` (god-table)
dissolution: what was dissolved, what the `G-DEVLOOP-DECOUPLE` ratchet measures, what the
legitimate remaining kernel is, and what genuine debt is left. It is the adversarially-built
SPEC (sshx thinking-triplet + ChatGPT Pro cross-model) referenced by the standing dissolution
goal, written as prose so the classification stays auditable and greppable.

## Two distinct coupling shapes — only one is the god-table anti-pattern

`libraries/devloop` capabilities reach a package's ambient `M` two ways, and they are **not**
the same thing:

1. **Copy-onto-M facade (the anti-pattern the ratchet targets).** A package core writes
   `M.fn = devloop_mod.fn` or `M.fn = function(...) return devloop_mod.fn(M, ...) end`, then
   department code reads `core.fn(...)`. This is the loophole the earlier
   install_defs/m_writes ratchet let slip (a wrapper facade drove those to 0 without
   decoupling). `check_repo_devloop_decouple.py` counts exactly these explicit-`M.name=`
   reader calls, and only rewiring a reader to a direct `require(module).fn(...)` lowers it.

2. **`install(M)` composed-core kernel (sanctioned).** A package core calls
   `require("devloop.logging").install(M)` / `require("devloop.state").install(M)`; those
   modules define `function M.<name>` inside the installer, and departments read
   `core.log_raise(...)` / `core.current_state(...)`. This is the composed core deliberately
   providing a small, stable set of cross-cutting + lifecycle capabilities to every
   department — the "**small documented version-CAS lifecycle kernel** [that] remains
   reachable through the composed core" named in the ratchet's own docstring. It is a
   Facade / shared-capability, not the god-table anti-pattern.

The ratchet measures shape (1) by design and does not count shape (2). That is correct: shape
(2) is the sanctioned kernel, not debt. An adversarial review initially read the uncounted
shape-(2) reads as hidden debt; the sshx thinking-triplet (minimal/structural/delete,
unanimous) and ChatGPT Pro (cross-model) resolved the fork per module (below): the bulk of
shape (2) is legitimate kernel, so leaving it uncounted is honest, not a false-negative.

## Facade coupling (shape 1): 659 → 27, dissolved

Explicit-`M.name=` facade coupling was driven from **659 to 27** across PRs #1777–#1807 by
genuine decoupling (rewire readers to direct `require`, drop vestigial `M`, direct-alias for
lower-layer library callers, whole-cluster drops for the lock-key name-collision family,
global-primitive threading for `exec_sync`/`core.git`). The residual 27:

- **25 non-devloop name-collisions** — counted only because a flat package binds a
  *same-named* symbol; the binding source proves it is not a `libraries/devloop` read:
  `read_env` 9 (`env.read_env`), `invalidate_entity_after_write` 7 (github-proxy's own
  `core/entity_view.lua`), `strip_bot_login_suffix` 4 (`forge_strings`), `trim` 2
  (integration-coverage-producer's own `function M.trim`), `judgment_codex_opts` 1
  (`workflow.codex`), `error_fingerprint` 1 (`contract.error_facts`), `git` 1 (`forge.git`).
  These are a measurement artifact of name collision, not god-table coupling.
- **2 genuine, forge-deferred** — `linked_pr_surface_snapshot` (github-devloop-pr,
  github-devloop) is `devloop.entity`-bound and genuine, but decoupling it needs
  `M.gh_pr_view_observe` (a GitHub egress capability) relocated to `libraries/forge`; it is a
  forge-port item, tracked with the egress debt below.

## `install(M)` kernel (shape 2): the documented composed-core kernel

Per the converged design, the `install(M)` capabilities split by module:

- **`devloop.state` = KERNEL** (~194 reads: `cas_outcome`, `stage_rank`,
  `state_label_changes`, `state_marker`, `current_state`, `versioned_transition_status`, …).
  These are the version-CAS lifecycle state machine — exactly the "version-CAS lifecycle
  kernel" the composed core is meant to provide. Small, stable surface per capability;
  coordinating the devloop lifecycle, not arbitrary department behaviour.
- **`devloop.logging` = KERNEL** (~731 reads: `log_cas_decision`, `log_raise`, `log_entry`,
  `log_apply`, `log_line`, …). Cross-cutting structured lifecycle logging. High fan-in is
  *expected* for logging and is not evidence of god-table coupling: the surface is a handful
  of stable primitives and the dependency direction is a leaf capability, not a tangle.
  Forcing every department to re-`require` logging would be uglier, not cleaner.
- **`devloop.commands` = DEBT (forge-port).** GitHub/git **egress** (e.g. `gh_pr_view_observe`,
  `gh_issue_view_observe`, git operations) belongs in `libraries/forge` per the repo's
  forge-port doctrine (all `gh`/`git` egress goes through `forge.github`/`forge.git`), not on
  the composed core. This is the genuine remaining ambient-`M` debt, to be relocated to the
  forge library. Its measured `core.*` fan-in through the composed core is small.

## Kernel guardrail — the composed core is not a new service locator

Classifying `devloop.state` + `devloop.logging` as a documented kernel is legitimate only while
that surface stays **small, stable, cohesive, and one-directional**. It is not a license to
copy arbitrary devloop functions onto `M` again — that would recreate the facade escape hatch
this whole effort removed. So the kernel is bounded by three rules:

- **Explicit + closed.** Only `devloop.state` (version-CAS lifecycle) and `devloop.logging`
  (structured lifecycle logging) install into the composed core as kernel. Any new `install(M)`
  capability, or any new symbol on those installers that is not lifecycle/logging, is treated as
  debt (either a direct `require(module).fn(...)` call site or a forge-port) until justified and
  added here deliberately — never a silent growth of the ambient surface.
- **No egress in the kernel.** GitHub/git egress never belongs on the composed core. The
  `devloop.commands` submodule egress (`gh_pr_view_observe`, `gh_issue_view_observe`, git
  operations) is debt to relocate into `libraries/forge`, not kernel to keep, regardless of
  fan-in.
- **One direction, boring.** Kernel capabilities are leaf lifecycle/logging primitives that
  departments consume; they must not grow into a general dependency hub. High fan-in on a tiny
  boring surface (logging) is fine; a wide or growing surface is the signal it is turning back
  into a god-table and must be split out.

## Endpoint

The god-table facade anti-pattern is dissolved (659 → 27 explicit facade reads, of which 25
are non-devloop name collisions and 2 are the forge-deferred `linked_pr_surface_snapshot`).
The remaining `install(M)` composed-core surface is the sanctioned kernel: `devloop.state`
(version-CAS lifecycle) and `devloop.logging` (cross-cutting structured logging), confirmed
kernel by an unanimous sshx thinking-triplet and ChatGPT Pro. The only genuine remaining
ambient-`M` debt is GitHub/git **egress** (`devloop.commands`) plus the egress-dependent
`linked_pr_surface_snapshot`, both of which are forge-port work: relocate the egress into
`libraries/forge` and thread the handle, after which the composed core carries only the
documented state + logging kernel.

⟦AI:FKST⟧
