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

# GitHub org owning the three repos.  default: ChronoAIProject
# GH_ORG=ChronoAIProject

# Which repos THIS machine drives ('all' and the board default expand to this list).
# A machine that only dogfoods two of the three repos lists just those.
# DOGFOOD_REPOS="packages substrate website"

# The github-devloop PLATFORM packages the supervise loads + runs from PKGSRC/packages/. REQUIRED:
# dogfood.sh is generic tooling and carries no package names, so this MUST be set here (run/restart
# fail-closed if unset). NOT every package in packages/ — the supervise RUNS packages (raisers fire),
# so it loads only the platform; independent agents (autochrono = issue->reply, archaudit = audit)
# must not co-run and fight over the same repo's issues. Add extracted packages here.
DEVLOOP_PKGS="github-devloop github-devloop-intake github-devloop-pr github-devloop-integration github-proxy consensus"

# STABLE durable roots — the redb persistent delivery store, REUSED across restarts so
# in-flight events resume. NEVER point these at a fresh path on a normal restart (that wipes
# the queue and strands mid-state issues). Pin the ACTUAL existing store path on this host:
# DUR_PACKAGES="$HOME/.fkst-dogfood/durable/packages"
# DUR_SUBSTRATE="$HOME/.fkst-dogfood/durable/substrate"
# DUR_WEBSITE="$HOME/.fkst-dogfood/durable/website"
