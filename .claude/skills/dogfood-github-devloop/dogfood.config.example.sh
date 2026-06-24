#!/usr/bin/env bash
# dogfood.config.example.sh — template for the per-machine dogfood config.
#
# We run THREE repos (fkst-packages, fkst-substrate, fkst-website) across TWO machines.
# `dogfood.sh` is identical on every host; this file holds what DIFFERS per machine. Copy it:
#
#     cp dogfood.config.example.sh dogfood.config.sh   # then edit for THIS host
#
# `dogfood.config.sh` is gitignored, so each machine keeps its own. Every value is OPTIONAL —
# dogfood.sh has a generic default for each (paths derive from $DOGFOOD_ROOT, BOT defaults to
# the gh auth user convention, branches default to dev/integration). Set only what differs here.
# Precedence: an explicit env var > this file > the built-in default.

# Base dir for all dogfood worktrees / logs / runtime scratch.  default: $HOME/.fkst-dogfood
# Keep it on a STABLE path. Do NOT use /private/tmp: macOS age-cleans it (files untouched >3d),
# which rots the run checkouts and the durable store.
# DOGFOOD_ROOT="$HOME/.fkst-dogfood"

# Substrate checkout the engine BIN builds from (BIN derives from it).  default: $HOME/fkst-substrate
# SUBSTRATE_SRC="$HOME/fkst-substrate"

# Trusted bot == THIS host's `gh auth` user. THE TWO MACHINES DIFFER HERE.
#   machine A:  BOT=loning
#   machine B:  BOT=ElonSG
# BOT=loning

# Per-device integration branch in the feature -> integration-<device> -> rollup -> dev flow.
#   machine A:  INTEGRATION_BRANCH=integration
#   machine B:  INTEGRATION_BRANCH=integration-elonsg
# INTEGRATION_BRANCH=integration

# Label prefix replayed by the generic github-proxy poller for this devloop deployment.
# GITHUB_PROXY_POLL_LABEL_PREFIX=fkst-dev:

# GitHub org owning the three repos.  default: ChronoAIProject
# GH_ORG=ChronoAIProject

# Which repos THIS machine drives ('all' and the board default expand to this list).
# A machine that only dogfoods two of the three repos lists just those.
# DOGFOOD_REPOS="packages substrate website"

# The github-devloop PLATFORM packages each supervise loads + runs from PKGSRC/packages/. The DEFAULT
# (the full platform: the github-devloop trio + the rest of the library; auto-audit DISABLED) now lives
# in dogfood.sh so every host stays consistent — you do NOT set it here unless THIS host genuinely
# differs. Precedence: env DEVLOOP_PKGS > this file > the dogfood.sh default.
#   WHERE TO LOOK (what packages exist + each one's role): PKGSRC/packages/<pkg>/ — `fkst.toml` gives
#     its `kind` (package | package.composed) and `[event_deps]`; `departments/<d>/main.lua` gives each
#     dept's `consumes`/`produces` (its event contract). That is the source of truth, not this list.
#   CO-RUN RULE (is a package safe to add to THIS platform supervise?): the supervise RUNS packages
#     (raisers fire), so an added agent must NOT contend with github-devloop for the same issues —
#     derive it from the consume surface. An issue-PRODUCER (consumes a non-issue signal — a cron tick,
#     system_idle — and produces github-proxy issue/comment requests) co-runs SAFELY: it only FILES
#     work that github-devloop's intake then judges (e.g. an architecture-audit agent). An issue-CONSUMER
#     (consumes github_entity_changed / claims + manages the issue lifecycle, e.g. an issue->reply agent)
#     WOULD fight github-devloop over the same issues and must run as its OWN separate supervise, not here.
#   AUTO-AUDIT is DISABLED: the archaudit + idle-detector audit-producer agents are NOT loaded on any
#     target (archaudit auto-filed engine SDK changes the pipeline could not safely develop). Re-enable
#     by adding `idle-detector archaudit` back to the dogfood.sh default DEVLOOP_PKGS if ever wanted.
#   OVERRIDE the whole platform list only if this host differs from the dogfood.sh default, e.g.:
#       # DEVLOOP_PKGS="github-devloop github-devloop-pr github-devloop-integration github-proxy consensus"

# STABLE durable roots — the redb persistent delivery store, REUSED across restarts so
# in-flight events resume. NEVER point these at a fresh path on a normal restart (that wipes
# the queue and strands mid-state issues). Pin the ACTUAL existing store path on this host:
# DUR_PACKAGES="$HOME/.fkst-dogfood/durable/packages"
# DUR_SUBSTRATE="$HOME/.fkst-dogfood/durable/substrate"
# DUR_WEBSITE="$HOME/.fkst-dogfood/durable/website"
