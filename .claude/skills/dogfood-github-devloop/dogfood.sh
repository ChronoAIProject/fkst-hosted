#!/usr/bin/env bash
# dogfood.sh — single operator multi-tool for dogfooding github-devloop on this device.
#
# Each dogfood drives ONE target repo's issue->PR->review->merge loop using the
# github-devloop behavior package (sourced from an fkst-packages clone), running on
# the fkst-substrate engine BIN with the integration->rollup->dev topology and real
# write posture (FKST_GITHUB_WRITE=1):
#
#   packages  : target fkst-packages   (host == package source: one worktree)
#   substrate : target fkst-substrate  (host = substrate engine worktree, packages from a sibling fkst-packages clone)
#   website   : target fkst-website    (host = website worktree + its own site-board package)
#
# Package layout: `.fkst/` is RUNTIME/build only (gitignored: runtime, durable,
# substrate-src, board cache) except host repos that intentionally commit their own
# local package source under `.fkst/local-packages/<pkg>`. The engine BIN + shared
# devloop packages are the PLATFORM, loaded by delegating the resolved topology to
# the host-run contract in `$PKGSRC/scripts/run.sh supervise`.
#
# Commands:
#   ./dogfood.sh status  [name|all]            pid/uptime/code-version/panic per supervise
#   ./dogfood.sh doctor  [name|all]            health roll-up: supervises + BIN freshness + code currency + graphql
#   ./dogfood.sh board   [name|all] [stale_h]  GitHub board sweep: which issues/PRs flow vs are stuck (default stale 6h)
#   ./dogfood.sh bin                           ensure engine BIN == substrate origin/dev; rebuild if stale (no restart)
#   ./dogfood.sh start   [name|all]            launch via host-run contract
#   ./dogfood.sh stop    [name|all]            SIGKILL (releases the redb lock)
#   ./dogfood.sh restart [name|all]            sync run checkouts to origin/<integration> + relaunch (unconditional)
#   ./dogfood.sh sync    [name|all]            auto-deploy: ff pinned operator checkouts to dev, rebuild BIN,
#                                              and restart ONLY supervises whose running code is a real
#                                              package/engine change (skill/docs-only skew is left running)
#   ./dogfood.sh logs    [name] [lines]        tail the latest log (default packages, 40 lines)
#
# Dogfood resolves per-machine topology and delegates one-host launch invariants
# (fresh runtime scratch, stable durable reuse, package loading, and restart) to
# `scripts/run.sh supervise`.
set -uo pipefail

# ---- per-machine config ----
# We run three repos (fkst-packages, fkst-substrate, fkst-website) across two machines.
# Device-specific values (paths, bot login, integration branch, stable durable roots, which
# repos this host drives) are sourced from a per-machine file so the SAME script runs anywhere:
#   1. $DOGFOOD_CONFIG if set, else  2. <this-dir>/dogfood.config.sh (gitignored, per-machine).
# Every value has a generic default below, so an unconfigured host still works. Precedence is
# env var > config file > default. See dogfood.config.example.sh for the template.
_self_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_cfg="${DOGFOOD_CONFIG:-$_self_dir/dogfood.config.sh}"
[ -f "$_cfg" ] && . "$_cfg"

DOGFOOD_ROOT="${DOGFOOD_ROOT:-$HOME/.fkst-dogfood}"                       # base for all worktrees/logs/scratch (STABLE $HOME path; NOT /private/tmp, which macOS age-cleans)
SUBSTRATE_SRC="${SUBSTRATE_SRC:-$HOME/fkst-substrate}"                    # substrate checkout the engine BIN builds from
BIN="${BIN:-$SUBSTRATE_SRC/target/debug/fkst-framework}"
RATE_POOL="${FKST_RATE_POOL_ROOT:-${RATE_POOL:-$DOGFOOD_ROOT/fkst-rate-pools}}"
BOT="${FKST_GITHUB_BOT_LOGIN:-${BOT:-loning}}"                            # gh auth user == trusted bot marker author (DIFFERS per machine)
LOGDIR="${DOGFOOD_LOGDIR:-${LOGDIR:-$DOGFOOD_ROOT}}"
UPSTREAM_BRANCH="${FKST_DEVLOOP_UPSTREAM_BRANCH:-${UPSTREAM_BRANCH:-dev}}"
INTEGRATION_BRANCH="${FKST_DEVLOOP_INTEGRATION_BRANCH:-${INTEGRATION_BRANCH:-integration}}"  # e.g. integration-<device> on a 2nd machine
ROLLUP_MERGE="${FKST_DEVLOOP_ROLLUP_MERGE:-${ROLLUP_MERGE:-auto}}"
MANAGED_BOT_LOGINS="${FKST_DEVLOOP_MANAGED_BOT_LOGINS:-${MANAGED_BOT_LOGINS:-}}"  # collaborating managed-bot logins (this device + peers); lets external-pr-intake skip our own automation
GITHUB_PROXY_POLL_LABEL_PREFIX="${FKST_GITHUB_PROXY_POLL_LABEL_PREFIX:-${GITHUB_PROXY_POLL_LABEL_PREFIX:-fkst-dev:}}"
GH_ORG="${GH_ORG:-ChronoAIProject}"
DOGFOOD_REPOS="${DOGFOOD_REPOS:-packages substrate website}"             # repos this host drives ('all' / board default expand here)

# The shared devloop trio = the PLATFORM (like GitHub runners + marketplace actions), loaded from the
# platform fkst-packages (Lua-primary) checkout's repo-root packages/ (PKGSRC). Each website-source-
# primary TARGET repo (host) commits its OWN custom Lua packages under `.fkst/local-packages/<pkg>`
# (root stays website source) — so the trio comes from `$PKGSRC/packages/<pkg>`, a host's own package
# from `$HOST/.fkst/local-packages/<pkg>`. (`.fkst/` is a tracked+ignored runtime INTERFACE dir, not
# "all runtime": host repos commit their own Lua there. See fkst-website CLAUDE.md.)
# Platform packages every dogfood supervise LOADS + RUNS from PKGSRC/packages/. The DEFAULT below is
# the full platform set (the github-devloop trio + the rest of the library) and lives HERE in the tool
# so every host is consistent; a host OVERRIDES it only when it genuinely differs (env DEVLOOP_PKGS >
# dogfood.config.sh > this default). The supervise loads only the platform (not every package in
# packages/) because it RUNS packages (raisers fire); co-loading independent agents would fight over
# the same repo's issues. (`test` loads all to validate the graph.) Auto-audit is DISABLED: the
# archaudit audit AGENT is NOT loaded on any target (re-add it here to re-enable). archaudit auditing
# the engine repo produced Rust SDK changes the pipeline cannot safely auto-develop (engine↔package
# contract, e.g. the proposal_id→dedup_key revert #174). idle-detector IS loaded though: it is NOT an
# audit agent but a shared PRODUCER the website's site-board depends on (board_scan consumes
# `idle-detector.system_idle`), so excluding it breaks the website supervise ("unknown namespace
# idle-detector" graph-scan failure); on packages/substrate it just produces an unconsumed
# system_idle with no consumer (harmless).
# integration-coverage-producer IS loaded: a co-run-safe issue-producer SCOPED to Lua-package run_graph
# coverage gaps (idle-gated + dedup'd + Lua-coverage-only, never engine) — the lasting self-drive arm of
# the integration-edge coverage ratchet; unlike archaudit it cannot file engine/SDK work.
DEVLOOP_PKGS="${DEVLOOP_PKGS:-github-devloop github-devloop-pr github-devloop-integration github-devloop-intake github-devloop-intake-default github-devloop-decompose github-devloop-ops github-proxy consensus github-external-pr-intake github-ratchet-migration-slicer fkst-substrate-ref-maintainer integration-coverage-producer idle-detector}"

# cfg <name> -> REPO HOST PKGSRC DUR LOCAL_PKGS. Worktree paths derive from $DOGFOOD_ROOT (uniform
# layout across machines); stable durable roots default under it but are commonly PINNED per machine
# (DUR_* in the config) to an existing redb store so restarts resume in-flight. LOCAL_PKGS lists the
# host target's OWN packages (committed under the host repo's `.fkst/local-packages/<pkg>`).
cfg() {
  LOCAL_PKGS=""   # repo-local custom packages this target drives (committed in the host repo)
  case "$1" in
    packages)
      REPO="$GH_ORG/fkst-packages"
      HOST="$DOGFOOD_ROOT/pkgs-dogfood"; PKGSRC="$HOST"
      DUR="${DUR_PACKAGES:-$DOGFOOD_ROOT/dogfood-durable-packages}" ;;
    substrate)
      REPO="$GH_ORG/fkst-substrate"
      HOST="$DOGFOOD_ROOT/substrate-dogfood/sub"; PKGSRC="$DOGFOOD_ROOT/substrate-dogfood/pkgs"
      DUR="${DUR_SUBSTRATE:-$DOGFOOD_ROOT/dogfood-durable-substrate}" ;;
    website)
      REPO="$GH_ORG/fkst-website"
      HOST="$DOGFOOD_ROOT/website-dogfood/site"; PKGSRC="$DOGFOOD_ROOT/website-dogfood/pkgs"
      DUR="${DUR_WEBSITE:-$DOGFOOD_ROOT/dogfood-durable-website}"; LOCAL_PKGS="site-board" ;;
    *) echo "unknown dogfood: $1 (packages|substrate|website)" >&2; return 1 ;;
  esac
}

pidof_df() { pgrep -f -- "supervise --project-root ${HOST} " 2>/dev/null; }
latest_log() { ls -t "$LOGDIR/${1}-sv-"*.log 2>/dev/null | head -1; }
pid_alive_non_zombie() {
  local pid="$1" stat
  kill -0 "$pid" 2>/dev/null || return 1
  stat=$(ps -o stat= -p "$pid" 2>/dev/null | awk 'NF {print $1; exit}')
  [[ "$stat" == Z* ]] && return 1
  return 0
}
supervise_ready_log() {
  local log="$1"
  # Supervise-owned startup readiness contract, not a generic health semantic.
  grep -qaE 'EVENT=code_provenance .*ENGINE_VER=[^ ]+ .*PKG_VERS=[^ ]+' "$log" 2>/dev/null \
    && grep -qa 'MSG=event runtime running' "$log" 2>/dev/null
}
wait_supervise_ready() { # $1 pid, $2 log
  local pid="$1" log="$2" attempts=0 ready_seen=0 stable_attempts=30 timeout_attempts=100
  while [ "$attempts" -lt "$timeout_attempts" ]; do
    if ! pid_alive_non_zombie "$pid"; then
      return 1
    fi
    if supervise_ready_log "$log"; then
      ready_seen=1
    fi
    if [ "$ready_seen" -eq 1 ] && [ "$attempts" -ge "$stable_attempts" ]; then
      return 0
    fi
    attempts=$((attempts + 1))
    sleep 0.1
  done
  return 2
}
# Parse an ISO-8601 UTC timestamp (trailing Z) to epoch. TZ=UTC is REQUIRED: BSD `date -j -f`
# ignores the Z and parses in the local zone, so on a +HH machine every computed age is inflated by
# the local UTC offset (e.g. +0800 -> board recency reads 8h too old -> healthy issues mislabelled
# "STUCK 8h"). now=`date +%s` is already zone-independent, so only the parse side needed fixing.
epoch_utc() { [ -z "${1:-}" ] && { echo 0; return; }; TZ=UTC date -j -f "%Y-%m-%dT%H:%M:%SZ" "$1" +%s 2>/dev/null || echo 0; }
expand() { [ "${1:-all}" = all ] && echo "$DOGFOOD_REPOS" || echo "$1"; }

issue_label_has() { # $1 comma-separated labels, $2 label
  case ",$1," in
    *",$2,"*) return 0 ;;
    *) return 1 ;;
  esac
}

issue_primary_state() { # $1 comma-separated fkst-dev labels
  local labels="$1" label state fallback="" old_ifs="$IFS"
  IFS=,
  for label in $labels; do
    [ -n "$label" ] || continue
    state="${label#fkst-dev:}"
    [ -n "$fallback" ] || fallback="$state"
    case "$state" in
      enabled|blocked-on-dependency) continue ;;
      *) IFS="$old_ifs"; echo "$state"; return 0 ;;
    esac
  done
  IFS="$old_ifs"
  echo "$fallback"
}

issue_recency_class() { # $1 issue-number, $2 labels, $3 state, $4 age-hours, $5 stale-hours, $6 open-pr-issue-numbers
  local num="$1" labels="$2" st="$3" age="$4" stale="$5" openpr="$6"
  case "$st" in
    tracking|pr-open) echo "tracking/umbrella" ;;
    blocked|impl-failed|merged|declined) echo "parked($st)" ;;
    thinking|ready|implementing|stalled-thinking)
      if [ "$st" = "ready" ] && issue_label_has "$labels" "fkst-dev:blocked-on-dependency"; then
        echo "parked(dependency-wait)"
      elif [ "$age" -ge "$stale" ]; then
        echo "⚠ STUCK $st ${age}h"
      else
        echo "✓ flowing $st ${age}h"
      fi
      ;;
    reviewing|fixing|review-meta|merge-ready|merging)
      if echo "$openpr" | grep -qx "$num"; then echo "$st →see PR (active)"; else echo "⚠ STRANDED $st (no open PR)"; fi
      ;;
    *) return 1 ;;
  esac
}

# Sync a dogfood RUN checkout (behavior PKGSRC + target HOST) to the machine's
# INTEGRATION_BRANCH — the dogfood runs its own pre-rollup code (feature ->
# integration-<device> -> rollup -> dev), so it is the live integration test of
# this device's autonomous changes BEFORE they promote to dev. The rollup target
# stays UPSTREAM_BRANCH (FKST_DEVLOOP_UPSTREAM_BRANCH=dev); only the engine BIN +
# the pinned operator/skill checkouts stay on dev.
# Self-heal a run checkout corrupted by a volatile DOGFOOD_ROOT. DOGFOOD_ROOT now defaults to a
# STABLE $HOME/.fkst-dogfood, but an explicit override to a volatile base like /private/tmp gets
# age-cleaned by macOS (files untouched >3d): it strips .git and older tracked
# files, leaving a partial tree. The constantly-written durable store survives, but the static
# package source rots — so the supervise either reads "skew/current" against the rotted checkout
# (operator fixes never deploy) or, on restart, refuses to start on an incomplete package graph
# (e.g. a raiser file gone -> "queue ... has no producer"). Detect that (no .git, or tracked files
# deleted) and re-clone fresh from origin. Forward-only restore; the durable store is separate and
# survives (or is re-derived from GitHub markers). $2 is the org/repo slug to clone (PKGSRC is
# always fkst-packages; HOST is the target $REPO). This makes correctness independent of where
# DOGFOOD_ROOT points, rather than relying on the base dir being non-volatile.
ensure_run_checkout() { # $1 checkout dir, $2 org/repo slug
  local dir="$1" slug="$2" corrupt=""
  if ! git -C "$dir" rev-parse --git-dir >/dev/null 2>&1; then
    corrupt="not-a-git-repo"
  elif git -C "$dir" status --porcelain 2>/dev/null | grep -q '^ D '; then
    corrupt="deleted-tracked-files"
  fi
  [ -z "$corrupt" ] && return 0
  echo "  ! run checkout $dir corrupt ($corrupt; likely $DOGFOOD_ROOT cleanup) -> re-cloning $slug"
  [ -e "$dir" ] && mv "$dir" "${dir}.corrupt.$(date +%s)" 2>/dev/null
  mkdir -p "$(dirname "$dir")"
  git clone -q "https://github.com/$slug.git" "$dir" \
    && echo "    re-cloned $slug -> $dir" \
    || { echo "    ERROR: failed to clone $slug into $dir"; return 1; }
}

sync_to_run_branch() { # $1 worktree dir
  git -C "$1" rev-parse --git-dir >/dev/null 2>&1 || { echo "  ! $1 is not a git worktree"; return 1; }
  git -C "$1" fetch origin "$INTEGRATION_BRANCH" -q 2>/dev/null
  local target; target=$(git -C "$1" rev-parse --short "origin/$INTEGRATION_BRANCH" 2>/dev/null)
  # checkout -B (not reset --hard): leaves the checkout actually ON the integration branch
  # tracking origin/<integration>, instead of pointing a stale local 'dev' ref at integration content.
  local note; note=$(git -C "$1" checkout -q -B "$INTEGRATION_BRANCH" "origin/$INTEGRATION_BRANCH" 2>&1 | tail -1)
  # Verify the checkout actually REACHED target, then self-heal. A checkout that aborts (working-tree
  # obstruction, a file<->symlink/dir transition racing the running supervise, a dirty tree) otherwise
  # leaves the clone on STALE code while the function returns ok and the supervise silently launches
  # stale — the exact "supervise silently re-running already-fixed defects" failure this tooling exists
  # to prevent. Self-heal forcefully (reset --hard + clean reaches the fetched ref regardless of the
  # obstruction; clean -fd keeps gitignored .fkst/ runtime), then re-assert the branch so the checkout
  # stays ON <integration>. If it STILL cannot reach target (deep corruption ensure_run_checkout should
  # have re-cloned), fail loud with STALE-CHECKOUT so the operator and doctor (pkg-stale) catch it.
  if [ -n "$target" ] && [ "$(git -C "$1" rev-parse --short HEAD 2>/dev/null)" != "$target" ]; then
    git -C "$1" reset --hard "origin/$INTEGRATION_BRANCH" -q 2>/dev/null
    git -C "$1" clean -fdq 2>/dev/null
    git -C "$1" checkout -q -B "$INTEGRATION_BRANCH" "origin/$INTEGRATION_BRANCH" 2>/dev/null
    note="self-healed stale checkout (was: ${note:-checkout-failed})"
  fi
  local head; head=$(git -C "$1" rev-parse --short HEAD 2>/dev/null)
  if [ -n "$target" ] && [ "$head" != "$target" ]; then
    echo "  $1 -> STALE-CHECKOUT: still $head, target $target ($note) ($INTEGRATION_BRANCH)"
    return 1
  fi
  echo "  $1 -> $head${note:+ ($note)} ($INTEGRATION_BRANCH)"
}

# Ensure a checkout's INTEGRATION_BRANCH is >= UPSTREAM_BRANCH (dev) by merging upstream
# FORWARD into integration and pushing. Why: operator out-of-band fixes land on dev; the
# dogfood runs on integration; the in-pipeline sync_scan ff's dev->integration but can lag
# (or the running supervise is itself stale), so _proc_stale reads "current" against a stale
# integration and the supervise never picks up operator fixes. This deterministically merges
# dev forward (plain ff when integration is an ancestor of dev; a merge commit when integration
# has its own un-rolled commits — both keep integration >= dev) and pushes, so the next
# _proc_stale sees pkg-stale and restarts onto the fix. Forward-only (never rewrites integration);
# aborts on conflict and leaves it for sync_conflict; a push failure is non-fatal.
ensure_integration_caught_up() { # $1 checkout dir
  local wt="$1"
  git -C "$wt" rev-parse --git-dir >/dev/null 2>&1 || return 0
  [ "$INTEGRATION_BRANCH" = "$UPSTREAM_BRANCH" ] && return 0   # single-branch topology: nothing to merge
  git -C "$wt" fetch origin "$INTEGRATION_BRANCH" "$UPSTREAM_BRANCH" -q 2>/dev/null || return 0
  git -C "$wt" rev-parse --verify "origin/$INTEGRATION_BRANCH" >/dev/null 2>&1 || return 0
  git -C "$wt" rev-parse --verify "origin/$UPSTREAM_BRANCH"   >/dev/null 2>&1 || return 0
  local behind; behind=$(git -C "$wt" rev-list --count "origin/$INTEGRATION_BRANCH..origin/$UPSTREAM_BRANCH" 2>/dev/null || echo 0)
  [ "${behind:-0}" -eq 0 ] && return 0
  echo "  $INTEGRATION_BRANCH is $behind behind $UPSTREAM_BRANCH in $(basename "$wt") -> merging $UPSTREAM_BRANCH forward"
  git -C "$wt" checkout -q -B "$INTEGRATION_BRANCH" "origin/$INTEGRATION_BRANCH" 2>/dev/null \
    || { echo "    WARN: could not checkout $INTEGRATION_BRANCH — leaving for sync_scan"; return 0; }
  if git -C "$wt" merge --no-edit "origin/$UPSTREAM_BRANCH" >/dev/null 2>&1; then
    if git -C "$wt" push origin "HEAD:$INTEGRATION_BRANCH" >/dev/null 2>&1; then
      echo "    merged + pushed: $INTEGRATION_BRANCH -> $(git -C "$wt" rev-parse --short HEAD)"
    else
      echo "    WARN: merge ok but push failed (perm/race) — leaving for sync_scan"
    fi
  else
    git -C "$wt" merge --abort 2>/dev/null
    echo "    WARN: $UPSTREAM_BRANCH does not merge cleanly into $INTEGRATION_BRANCH — leaving for sync_conflict"
  fi
}

# Engine BIN freshness. Stale = substrate origin/dev ahead of the build checkout, OR any
# crate .rs newer than the BIN binary. _bin_state echoes "behind newer head" (read-only,
# fetches first) and is shared by the read-only report and the rebuild.
_bin_state() {
  git -C "$SUBSTRATE_SRC" fetch origin "$UPSTREAM_BRANCH" -q 2>/dev/null
  local behind newer head
  head=$(git -C "$SUBSTRATE_SRC" rev-parse --short HEAD 2>/dev/null)
  behind=$(git -C "$SUBSTRATE_SRC" rev-list --count "HEAD..origin/$UPSTREAM_BRANCH" 2>/dev/null || echo 0)
  newer=$(find "$SUBSTRATE_SRC/crates" -name '*.rs' -newer "$BIN" 2>/dev/null | wc -l | tr -d ' ')
  echo "${behind:-0} ${newer:-0} ${head:-?}"
}

# Read-only freshness report — used by `doctor`, which must NOT mutate. Rebuilding here would
# make the BIN file current while the RUNNING supervise still executes the old engine, masking
# the staleness behind a "fresh" line. So doctor only reports; `restart`/`bin` rebuild + reload.
bin_freshness_report() {
  git -C "$SUBSTRATE_SRC" rev-parse --git-dir >/dev/null 2>&1 || { echo "$SUBSTRATE_SRC not a substrate checkout"; return 0; }
  local behind newer head; read -r behind newer head <<<"$(_bin_state)"
  if [ -x "$BIN" ] && [ "$behind" = 0 ] && [ "$newer" = 0 ]; then
    echo "fresh: substrate@$head (0 behind origin/$UPSTREAM_BRANCH, 0 newer .rs)"
  else
    echo "STALE: substrate@$head behind=$behind newer_rs=$newer → run 'dogfood.sh restart' (or 'bin') to rebuild + reload"
  fi
}

# Rebuild the BIN from substrate origin/dev if stale — used by bin/sync (mutating).
bin_ensure_fresh() {
  git -C "$SUBSTRATE_SRC" rev-parse --git-dir >/dev/null 2>&1 \
    || { echo "BIN: $SUBSTRATE_SRC not a substrate checkout — skipping freshness check"; return 0; }
  local behind newer head; read -r behind newer head <<<"$(_bin_state)"
  if [ -x "$BIN" ] && [ "$behind" = 0 ] && [ "$newer" = 0 ]; then
    echo "BIN fresh: substrate@$head (0 behind origin/$UPSTREAM_BRANCH, 0 newer .rs)"
    return 0
  fi
  echo "BIN STALE (behind=$behind newer_rs=$newer) — rebuild from origin/$UPSTREAM_BRANCH"
  if [ -n "$(git -C "$SUBSTRATE_SRC" status --porcelain 2>/dev/null)" ]; then
    echo "  ! $SUBSTRATE_SRC working tree dirty — building current HEAD without reset:"
    git -C "$SUBSTRATE_SRC" status --short | head -5 | sed 's/^/    /'
  else
    git -C "$SUBSTRATE_SRC" reset --hard "origin/$UPSTREAM_BRANCH" 2>&1 | tail -1 | sed 's/^/  /'
  fi
  ( cd "$SUBSTRATE_SRC" && cargo build -p fkst-framework 2>&1 | tail -2 | sed 's/^/  /' )
  echo "  built: substrate@$(git -C "$SUBSTRATE_SRC" rev-parse --short HEAD)"
}

# Prune worktrees + scratch dirs from OLD runtime roots of this dogfood (implement/fix
# depts create worktrees under the launch runtime scratch, registered in the shared .git; each
# restart makes a fresh runtime root, orphaning the old registrations — registry leak #500).
clean_stale_runtime_worktrees() { # $1 name, $2 current-rt-to-keep
  local name="$1" keep="$2" wt d
  git -C "$PKGSRC" worktree list --porcelain 2>/dev/null | awk '/^worktree /{print $2}' \
    | grep -F "/dogfood-rt-${name}." | grep -vF "$keep" \
    | while read -r wt; do git -C "$PKGSRC" worktree remove --force "$wt" 2>/dev/null; done
  git -C "$PKGSRC" worktree prune 2>/dev/null
  for d in "$LOGDIR"/dogfood-rt-"${name}".*; do
    [ -d "$d" ] && [ "$d" != "$keep" ] && rm -rf "$d" 2>/dev/null
  done
}

launch_one() { # $1 name, $2 restart flag (0|1)
  local name="$1" restart="${2:-0}" ts log rt args=()
  ts=$(date +%s); log="$LOGDIR/${name}-sv-${ts}.log"; rt="$LOGDIR/dogfood-rt-${name}.${ts}"
  clean_stale_runtime_worktrees "$name" "$rt"
  [ -n "$DEVLOOP_PKGS" ] || { echo "[$name] DEVLOOP_PKGS unset — set the platform packages to load in dogfood.config.sh (see dogfood.config.example.sh)"; return 1; }
  [ -x "$PKGSRC/scripts/run.sh" ] || { echo "[$name] missing host-run contract: $PKGSRC/scripts/run.sh"; return 1; }

  args=(
    "$PKGSRC/scripts/run.sh" supervise
    --project-root "$HOST"
    --platform-root "$PKGSRC"
    --platform-packages "$DEVLOOP_PKGS"
    --durable-root "$DUR"
    --runtime-root "$rt"
  )
  [ -n "$LOCAL_PKGS" ] && args+=(--host-packages "$LOCAL_PKGS")
  [ "$restart" = "1" ] && args+=(--restart)

  BIN="$BIN" FKST_GITHUB_REPO="$REPO" FKST_GITHUB_WRITE=1 FKST_GITHUB_BOT_LOGIN="$BOT" \
    FKST_GITHUB_PROXY_POLL_LABEL_PREFIX="$GITHUB_PROXY_POLL_LABEL_PREFIX" \
    FKST_DEVLOOP_UPSTREAM_BRANCH="$UPSTREAM_BRANCH" FKST_DEVLOOP_INTEGRATION_BRANCH="$INTEGRATION_BRANCH" \
    FKST_DEVLOOP_ROLLUP_MERGE="$ROLLUP_MERGE" FKST_DEVLOOP_MANAGED_BOT_LOGINS="$MANAGED_BOT_LOGINS" \
    FKST_RATE_POOL_ROOT="$RATE_POOL" \
    nohup "${args[@]}" > "$log" 2>&1 &
  local pid=$!
  ln -sf "$log" "$LOGDIR/${name}-sv.log"
  wait_supervise_ready "$pid" "$log"
  local ready_status=$?
  if [ "$ready_status" -eq 0 ]; then
    echo "[$name] started pid $pid  panic=$(grep -ac panicked "$log" 2>/dev/null)  log=$log"
  else
    if [ "$ready_status" -eq 1 ]; then
      echo "[$name] FAILED to start; supervise pid $pid exited before readiness; tail:"
    else
      echo "[$name] FAILED to become ready; supervise pid $pid did not emit startup readiness; tail:"
    fi
    tail -12 "$log" | sed 's/\x1b\[[0-9;]*m//g'
    return 1
  fi
}

start_one() {
  cfg "$1" || return 1
  local existing; existing=$(pidof_df)
  if [ -n "$existing" ]; then echo "[$1] already running (pid $existing) — use restart"; return 0; fi
  launch_one "$1" 0
}

stop_one() {
  cfg "$1" || return 1
  local p; p=$(pidof_df)
  if [ -z "$p" ]; then echo "[$1] not running"; return 0; fi
  kill -9 $p 2>/dev/null; echo "[$1] killed $p"
}

restart_one() {
  cfg "$1" || return 1
  echo "[$1] sync to origin/$INTEGRATION_BRANCH (run branch; rollup target stays $UPSTREAM_BRANCH):"
  ensure_run_checkout "$PKGSRC" "$GH_ORG/fkst-packages"              # re-clone if DOGFOOD_ROOT cleanup rotted the checkout
  [ "$HOST" != "$PKGSRC" ] && ensure_run_checkout "$HOST" "$REPO"
  ensure_integration_caught_up "$PKGSRC"                              # keep run branch (integration) >= dev so operator fixes deploy
  [ "$HOST" != "$PKGSRC" ] && ensure_integration_caught_up "$HOST"
  sync_to_run_branch "$PKGSRC"
  [ "$HOST" != "$PKGSRC" ] && sync_to_run_branch "$HOST"
  # One migration bridge: a supervise launched before the host-run contract has no
  # durable pidfile yet, so --restart has nothing to kill on the first upgraded run.
  [ ! -f "$DUR/.fkst-supervise.pid" ] && { stop_one "$1"; sleep 1; }
  launch_one "$1" 1
}

status_one() {
  cfg "$1" || return 1
  local p log; p=$(pidof_df); log=$(latest_log "$1")
  if [ -z "$p" ]; then echo "[$1] STOPPED   (target $REPO)"; return 0; fi
  local et panic last hv pv
  et=$(ps -o etime= -p $p 2>/dev/null | tr -d ' ')
  panic=$(grep -ciE "thread '[^']*' panicked|panicked at|redb.*lock error" "$log" 2>/dev/null)
  last=$(tail -1 "$log" 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' | cut -c1-44)
  hv=$(git -C "$HOST" rev-parse HEAD 2>/dev/null | cut -c1-8)
  pv=$(git -C "$PKGSRC" rev-parse HEAD 2>/dev/null | cut -c1-8)
  printf '[%s] RUNNING pid %s up %s panic=%s | host@%s pkgs@%s | %s\n' "$1" "$p" "$et" "$panic" "$hv" "$pv" "$last"
}

# _proc_stale <name> -> freshness verdict of the RUNNING process vs origin/dev. Authoritative =
# the code the process loaded at startup (logged code_provenance PKG_VERS/ENGINE_VER), NOT the
# worktree/BIN file (those can be updated without reloading the process — only a restart reloads).
# Echoes: stopped | current | skew (dev moved, non-package files only) | pkg-stale | engine-stale.
# PKG freshness is vs PKGSRC origin/$INTEGRATION_BRANCH (the run branch the dogfood loads);
# ENGINE freshness is vs SUBSTRATE_SRC origin/$UPSTREAM_BRANCH (the BIN is built from dev).
# Side effect: fetches origin/$INTEGRATION_BRANCH for $PKGSRC and origin/$UPSTREAM_BRANCH for $SUBSTRATE_SRC.
_proc_stale() {
  cfg "$1" || { echo unknown; return; }
  local p log procpkg proceng pdev sdev; p=$(pidof_df); log=$(latest_log "$1")
  [ -z "$p" ] && { echo stopped; return; }
  git -C "$PKGSRC" fetch origin "$INTEGRATION_BRANCH" -q 2>/dev/null
  git -C "$SUBSTRATE_SRC" fetch origin "$UPSTREAM_BRANCH" -q 2>/dev/null
  pdev=$(git -C "$PKGSRC" rev-parse "origin/$INTEGRATION_BRANCH" 2>/dev/null)
  sdev=$(git -C "$SUBSTRATE_SRC" rev-parse "origin/$UPSTREAM_BRANCH" 2>/dev/null)
  procpkg=$(grep -aoE "${DEVLOOP_PKGS%% *}@[a-f0-9]+" "$log" 2>/dev/null | tail -1 | cut -d@ -f2)   # any platform pkg's commit reflects the running code
  proceng=$(grep -aoE 'ENGINE_VER=[a-f0-9]+' "$log" 2>/dev/null | tail -1 | cut -d= -f2)
  if [ -n "$proceng" ] && [ "${sdev:0:${#proceng}}" != "$proceng" ]; then echo engine-stale; return; fi
  if [ -n "$procpkg" ] && [ "${pdev:0:${#procpkg}}" != "$procpkg" ]; then
    if [ -n "$(git -C "$PKGSRC" diff "$procpkg" "$pdev" -- packages/ 2>/dev/null)" ]; then echo pkg-stale; else echo skew; fi
    return
  fi
  echo current
}

doctor_one() {
  cfg "$1" || return 1
  local p log panic st procpkg proceng verdict; p=$(pidof_df); log=$(latest_log "$1")
  panic=$(grep -ac panicked "$log" 2>/dev/null); panic=${panic:-0}
  if [ -z "$p" ]; then printf '  %-9s STOPPED (target %s)\n' "$1" "$REPO"; return 0; fi
  st=$(_proc_stale "$1")   # also fetches origin/dev for $PKGSRC + $SUBSTRATE_SRC
  procpkg=$(grep -aoE "${DEVLOOP_PKGS%% *}@[a-f0-9]+" "$log" 2>/dev/null | tail -1 | cut -d@ -f2)
  proceng=$(grep -aoE 'ENGINE_VER=[a-f0-9]+' "$log" 2>/dev/null | tail -1 | cut -d= -f2)
  case "$st" in
    current)      verdict="pkg-current engine-current" ;;
    skew)         verdict="pkg-skew(non-package, no restart) engine-current" ;;
    pkg-stale)    verdict="PKG-STALE→restart(${procpkg:0:8}≠$(git -C "$PKGSRC" rev-parse --short "origin/$INTEGRATION_BRANCH" 2>/dev/null))" ;;
    engine-stale) verdict="ENGINE-STALE→restart(${proceng:0:8}≠$(git -C "$SUBSTRATE_SRC" rev-parse --short "origin/$UPSTREAM_BRANCH" 2>/dev/null))" ;;
    *)            verdict="$st" ;;
  esac
  printf '  %-9s RUNNING pid %s up %s | %s | worktree %s | panic %s\n' "$1" "$p" \
    "$(ps -o etime= -p $p 2>/dev/null|tr -d ' ')" "$verdict" "$(git -C "$PKGSRC" rev-parse --short HEAD 2>/dev/null)" "$panic"
}

# durable_health_one <name>: surface redb delivery-queue state (stuck-pending events + dead-letters)
# that the supervise-LOG scan is structurally blind to — a stuck pending delivery or a dead-letter is
# durable-queue state, not a log line. `observe` is the engine's authoritative durable-state aggregator,
# so reuse it (don't reimplement). Flags ⚠ on any dead-letter or a pending event older than 6h (the
# board's stale threshold). Reads a live supervise's redb via a single read transaction (no lock fight).
durable_health_one() {
  cfg "$1" || return 0
  if [ ! -e "$DUR/delivery.redb" ]; then echo "  $1: no durable store"; return 0; fi
  local summary
  summary=$("$BIN" observe --json --durable-root "$DUR" 2>/dev/null | jq -r '
    ([.queues[].pending]|add // 0) as $p |
    (([.queues[].oldest_pending_age_ms]|max // 0)/3600000|floor) as $oh |
    (.dead_letters|length) as $dl |
    "\(.queues|length) queues, \($p) pending (oldest \($oh)h), \($dl) dead-letters"
      + (if ($dl>0 or $oh>6) then " ⚠" else "" end)' 2>/dev/null)
  echo "  $1: ${summary:-observe unavailable}"
}

cmd_doctor() {
  echo "engine BIN:"; bin_freshness_report | sed 's/^/  /'
  echo "supervises:"
  for n in $(expand "${1:-all}"); do doctor_one "$n"; done
  echo "upstream($UPSTREAM_BRANCH) CI:"; for n in $(expand "${1:-all}"); do upstream_ci_one "$n"; done
  echo "durable (redb delivery state):"; for n in $(expand "${1:-all}"); do durable_health_one "$n"; done
  echo "graphql: $(gh api rate_limit --jq '.resources.graphql.remaining' 2>/dev/null||echo ?)/5000"
}

# upstream_ci_one <name>: report the latest `ci` conclusion on UPSTREAM_BRANCH (dev) for the
# target's repo. The supervise gates merges on feature/integration CI, NOT on the dev-push run,
# so a dev-push CI that goes red is INVISIBLE to the board's per-PR view — yet a permanently-red
# dev CI masks real regressions (you cannot tell a genuine dev break from a chronic one). This
# line is the operator's trustworthiness check on the dev gate itself. REST (`gh run list`), so it
# does not compete with the pipeline's graphql budget.
upstream_ci_one() {
  cfg "$1" 2>/dev/null || return 0
  local row concl sha
  row=$(gh run list --repo "$REPO" --branch "$UPSTREAM_BRANCH" --workflow ci.yml --event push \
        --limit 1 --json conclusion,headSha -q '.[]|"\(.conclusion) \(.headSha[0:8])"' 2>/dev/null)
  concl=${row%% *}; sha=${row##* }
  case "$concl" in
    success) printf '  %-9s green (%s)\n' "$1" "$sha" ;;
    "")      printf '  %-9s (no push CI runs)\n' "$1" ;;
    *)       printf '  %-9s ⚠ %s (%s) — %s CI NOT green; real regression or workflow defect, investigate\n' \
               "$1" "$concl" "$sha" "$UPSTREAM_BRANCH" ;;
  esac
}

# _sync_checkout <dir>: fast-forward a pinned operator checkout to origin/$UPSTREAM_BRANCH. Only
# touches a CLEAN checkout whose HEAD is an ancestor of origin/dev (a behind-but-on-the-dev-line
# mirror); refuses a dirty tree or a feature-branch/diverged checkout (those are worktrees doing
# work — never reset them). A pinned operator checkout must stay a clean dev mirror; make changes
# in a worktree (see SKILL.md).
_sync_checkout() {
  local co="$1" before after
  { [ -n "$co" ] && git -C "$co" rev-parse --git-dir >/dev/null 2>&1; } || { echo "  ${co:-?}: not a git checkout (skip)"; return; }
  git -C "$co" fetch origin "$UPSTREAM_BRANCH" -q 2>/dev/null
  before=$(git -C "$co" rev-parse --short HEAD 2>/dev/null)
  if [ -n "$(git -C "$co" status --porcelain 2>/dev/null)" ]; then
    echo "  $co: DIRTY — not synced (a pinned checkout must be clean; make changes in a worktree)"; return
  fi
  if ! git -C "$co" merge-base --is-ancestor HEAD "origin/$UPSTREAM_BRANCH" 2>/dev/null; then
    echo "  $co: $before not an ancestor of origin/$UPSTREAM_BRANCH — skip (feature branch / diverged; not a pinned dev mirror)"; return
  fi
  git -C "$co" reset --hard "origin/$UPSTREAM_BRANCH" -q 2>/dev/null
  after=$(git -C "$co" rev-parse --short HEAD 2>/dev/null)
  [ "$before" = "$after" ] && echo "  $co: current ($after)" || echo "  $co: $before -> $after"
}

# cmd_sync: keep everything current in one call. Fast-forward the pinned operator checkouts (the one
# this skill+dogfood.sh load from, and the substrate BIN source) to origin/dev, rebuild the BIN if
# the engine moved, then AUTO-RESTART only the supervises whose RUNNING code is a real package or
# engine change (pkg-stale/engine-stale). Skill/docs-only skew and already-current processes are
# left running — a restart would only churn in-flight codex for no code change.
cmd_sync() {
  echo "operator checkouts -> origin/$UPSTREAM_BRANCH:"
  _sync_checkout "$(git -C "$_self_dir" rev-parse --show-toplevel 2>/dev/null)"  # repo this skill lives in
  _sync_checkout "$SUBSTRATE_SRC"                                                # engine BIN source
  echo "engine BIN:"; bin_ensure_fresh | sed 's/^/  /'
  echo "supervises (auto-restart only on real code change):"
  local n st failed=0
  for n in $(expand "${1:-all}"); do
    cfg "$n" || continue
    ensure_run_checkout "$PKGSRC" "$GH_ORG/fkst-packages"              # re-clone if DOGFOOD_ROOT cleanup rotted the checkout (else _proc_stale misreads "skew" and fixes never deploy)
    [ "$HOST" != "$PKGSRC" ] && ensure_run_checkout "$HOST" "$REPO"
    ensure_integration_caught_up "$PKGSRC"                              # keep run branch (integration) >= dev so operator fixes deploy
    [ "$HOST" != "$PKGSRC" ] && ensure_integration_caught_up "$HOST"
    st=$(_proc_stale "$n")
    case "$st" in
      pkg-stale|engine-stale) echo "  $n: $st -> auto-restart"; restart_one "$n" | sed 's/^/    /' || failed=1 ;;
      stopped)                echo "  $n: stopped (use 'start' to launch)" ;;
      *)                      echo "  $n: $st (no restart needed)" ;;
    esac
  done
  return "$failed"
}

board_one() { # $1 name, $2 stale_hours
  cfg "$1" || return 1
  local stale="$2" now; now=$(date +%s)
  echo "════════════════════════════════════════ $REPO"
  local p; p=$(pidof_df)
  echo "supervise: $([ -n "$p" ] && echo "pid $p up $(ps -o etime= -p $p 2>/dev/null|tr -d ' ')" || echo 'NOT RUNNING locally') | graphql $(gh api rate_limit --jq '.resources.graphql.remaining' 2>/dev/null||echo ?)/5000"
  local openpr; openpr=$(gh api "repos/$REPO/pulls?state=open&per_page=100" --jq '.[]|.head.ref' 2>/dev/null | grep -oE '/[0-9]+/' | tr -d '/' | sort -u)
  echo "── PRs (active work · CI · recency) ──"
  gh api "repos/$REPO/pulls?state=open&per_page=100" --jq '.[]|"\(.number)\t\(.head.sha[0:8])\t\(.updated_at)\t\(.base.ref)\t\(.title[0:42])"' 2>/dev/null | \
  while IFS=$'\t' read -r num sha upd base title; do
    local chk a flow; chk=$(gh api "repos/$REPO/commits/$sha/check-runs" --jq '[.check_runs[]|select(.name|test("CodeQL")|not)|.conclusion//.status]|join(",")' 2>/dev/null)
    a=$(( (now - $(epoch_utc "$upd")) / 3600 ))
    if   echo "$chk"|grep -qE 'failure|cancelled'; then flow="⚠ CI-RED"
    elif [ -z "$chk" ];                              then flow="⚠ NO-CI"
    elif [ "$a" -ge $((stale*2)) ];                  then flow="⚠ STUCK ${a}h"
    else flow="✓ flowing ${a}h"; fi
    printf "  PR#%-4s →%-12s %-12s %s\n" "$num" "$base" "$flow" "$title"
  done
  echo "── issues (by fkst-dev state) ──"
  gh api "repos/$REPO/issues?state=open&per_page=100" --jq '.[]|select(.pull_request==null)|([.labels[].name]|map(select(startswith("fkst-dev:")and .!="fkst-dev:enabled"))) as $labels|"\(.number)\t\(.updated_at)\t\(if ($labels|length)==0 then "__fkst_stateless__" else ($labels|join(",")) end)\t\(.title[0:38])"' 2>/dev/null | \
  while IFS=$'\t' read -r num upd label title; do
    local a st cls; a=$(( (now - $(epoch_utc "$upd")) / 3600 )); st="$(issue_primary_state "$label")"
    if [ -z "$label" ] || [ "$label" = "__fkst_stateless__" ]; then
      st="stateless"
      if [ "$a" -ge "$stale" ]; then cls="⚠ STRANDED stateless ${a}h"; else cls="✓ waiting intake ${a}h"; fi
    else
      cls="$(issue_recency_class "$num" "$label" "$st" "$a" "$stale" "$openpr")" || continue
    fi
    printf "  #%-4s [%-12s] %s\n" "$num" "$st" "$cls"
  done
  echo ""
}

cmd_config() {
  echo "resolved config ($([ -f "$_cfg" ] && echo "from $_cfg" || echo 'defaults only — no per-machine config file'))"
  printf '  %-18s %s\n' DOGFOOD_ROOT "$DOGFOOD_ROOT" SUBSTRATE_SRC "$SUBSTRATE_SRC" BIN "$BIN" \
    BOT "$BOT" GH_ORG "$GH_ORG" UPSTREAM_BRANCH "$UPSTREAM_BRANCH" INTEGRATION_BRANCH "$INTEGRATION_BRANCH" \
    ROLLUP_MERGE "$ROLLUP_MERGE" RATE_POOL "$RATE_POOL" LOGDIR "$LOGDIR" DOGFOOD_REPOS "$DOGFOOD_REPOS"
  echo "platform pkgs (DEVLOOP_PKGS, from each PKGSRC/packages): $DEVLOOP_PKGS"
  echo "per-repo (HOST | PKGSRC | DURABLE | local pkgs):"
  local n; for n in $DOGFOOD_REPOS; do cfg "$n" && printf '  %-9s %s | %s | %s | %s\n' "$n" "$HOST" "$PKGSRC" "$DUR" "${LOCAL_PKGS:--}"; done
}

cmd_board() {
  local target="${1:-}" stale="${2:-6}"
  # accept `board <stale_hours>` (numeric first arg) as well as `board [name] [stale_hours]`
  if [ -n "$target" ] && [ -z "${target//[0-9]/}" ]; then stale="$target"; target=""; fi
  [ -z "$target" ] && target="$DOGFOOD_REPOS" || target=$(expand "$target")
  for n in $target; do board_one "$n" "$stale"; done
  echo "✓ flowing / tracking / parked = ok   ·   ⚠ STUCK/STRANDED/CI-RED/NO-CI = needs attention (stale=${stale}h)"
  echo "(label-based fast view; for authoritative state cross-check the issue's state:v1 marker / the linked PR)"
}

cmd="${1:-status}"; arg2="${2:-}"; arg3="${3:-}"
case "$cmd" in
  bin)     bin_ensure_fresh ;;
  start)   rc=0; for n in $(expand "${arg2:-all}"); do start_one "$n" || rc=1; done; exit "$rc" ;;
  stop)    for n in $(expand "${arg2:-all}"); do stop_one "$n"; done ;;
  restart) rc=0; for n in $(expand "${arg2:-all}"); do restart_one "$n" || rc=1; done; exit "$rc" ;;
  sync)    cmd_sync "$arg2" ;;
  status)  for n in $(expand "${arg2:-all}"); do status_one "$n"; done ;;
  doctor)  cmd_doctor "${arg2:-all}" ;;
  config)  cmd_config ;;
  board)   cmd_board "$arg2" "$arg3" ;;
  logs)    f=$(latest_log "${arg2:-packages}"); echo "$f"; tail -"${arg3:-40}" "$f" 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' ;;
  *) echo "usage: $0 {status|doctor|config|board|bin|start|stop|restart|sync|logs} [packages|substrate|website|all] [stale_h|lines]"; exit 1 ;;
esac
