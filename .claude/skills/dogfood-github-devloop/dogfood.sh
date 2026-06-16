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
# Commands:
#   ./dogfood.sh status  [name|all]            pid/uptime/code-version/panic per supervise
#   ./dogfood.sh doctor  [name|all]            health roll-up: supervises + BIN freshness + code currency + graphql
#   ./dogfood.sh board   [name|all] [stale_h]  GitHub board sweep: which issues/PRs flow vs are stuck (default stale 6h)
#   ./dogfood.sh bin                           ensure engine BIN == substrate origin/dev; rebuild if stale (no restart)
#   ./dogfood.sh start   [name|all]            launch (ensures BIN fresh first)
#   ./dogfood.sh stop    [name|all]            SIGKILL (releases the redb lock)
#   ./dogfood.sh restart [name|all]            BIN-fresh + sync worktrees to origin/dev + SIGKILL + relaunch
#   ./dogfood.sh logs    [name] [lines]        tail the latest log (default packages, 40 lines)
#
# Two invariants this script encodes so each dogfood wake is a few clean calls, not ad-hoc bash:
#  - Restart-to-deploy (CLAUDE.md): FKST_RUNTIME_ROOT is scratch (fresh each launch);
#    FKST_DURABLE_ROOT is the redb persistent store and is REUSED across restarts so
#    durable in-flight events resume. A fresh durable root would strand mid-state issues.
#  - BIN freshness: a direct `BIN supervise` launch does NOT auto-build like
#    `scripts/run.sh` does, so the BIN silently goes stale when substrate dev moves (an
#    engine .rs fix merges). start/restart ensure BIN == substrate origin/dev first.
#    BIN is exported (not only --framework-bin) so spawned implement/fix codex can run the suite.
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

DOGFOOD_ROOT="${DOGFOOD_ROOT:-/private/tmp}"                              # base for all worktrees/logs/scratch
SUBSTRATE_SRC="${SUBSTRATE_SRC:-$HOME/fkst-substrate}"                    # substrate checkout the engine BIN builds from
BIN="${BIN:-$SUBSTRATE_SRC/target/debug/fkst-framework}"
RATE_POOL="${FKST_RATE_POOL_ROOT:-${RATE_POOL:-$DOGFOOD_ROOT/fkst-rate-pools}}"
BOT="${FKST_GITHUB_BOT_LOGIN:-${BOT:-loning}}"                            # gh auth user == trusted bot marker author (DIFFERS per machine)
LOGDIR="${DOGFOOD_LOGDIR:-${LOGDIR:-$DOGFOOD_ROOT}}"
UPSTREAM_BRANCH="${FKST_DEVLOOP_UPSTREAM_BRANCH:-${UPSTREAM_BRANCH:-dev}}"
INTEGRATION_BRANCH="${FKST_DEVLOOP_INTEGRATION_BRANCH:-${INTEGRATION_BRANCH:-integration}}"  # e.g. integration-<device> on a 2nd machine
ROLLUP_MERGE="${FKST_DEVLOOP_ROLLUP_MERGE:-${ROLLUP_MERGE:-auto}}"
GH_ORG="${GH_ORG:-ChronoAIProject}"
DOGFOOD_REPOS="${DOGFOOD_REPOS:-packages substrate website}"             # repos this host drives ('all' / board default expand here)

# cfg <name> -> REPO HOST PKGSRC DUR EXTRA. Worktree paths derive from $DOGFOOD_ROOT (uniform
# layout across machines); stable durable roots default under it but are commonly PINNED per
# machine (DUR_* in the config) to an existing redb store so restarts resume in-flight.
cfg() {
  case "$1" in
    packages)
      REPO="$GH_ORG/fkst-packages"
      HOST="$DOGFOOD_ROOT/pkgs-dogfood"; PKGSRC="$HOST"
      DUR="${DUR_PACKAGES:-$DOGFOOD_ROOT/dogfood-durable-packages}"; EXTRA="" ;;
    substrate)
      REPO="$GH_ORG/fkst-substrate"
      HOST="$DOGFOOD_ROOT/substrate-dogfood/sub"; PKGSRC="$DOGFOOD_ROOT/substrate-dogfood/pkgs"
      DUR="${DUR_SUBSTRATE:-$DOGFOOD_ROOT/dogfood-durable-substrate}"; EXTRA="" ;;
    website)
      REPO="$GH_ORG/fkst-website"
      HOST="$DOGFOOD_ROOT/website-dogfood/site"; PKGSRC="$DOGFOOD_ROOT/website-dogfood/pkgs"
      DUR="${DUR_WEBSITE:-$DOGFOOD_ROOT/dogfood-durable-website}"; EXTRA="$HOST/packages/site-board" ;;
    *) echo "unknown dogfood: $1 (packages|substrate|website)" >&2; return 1 ;;
  esac
}

pidof_df() { pgrep -f -- "supervise --project-root ${HOST} " 2>/dev/null; }
latest_log() { ls -t "$LOGDIR/${1}-sv-"*.log 2>/dev/null | head -1; }
epoch_utc() { [ -z "${1:-}" ] && { echo 0; return; }; date -j -f "%Y-%m-%dT%H:%M:%SZ" "$1" +%s 2>/dev/null || echo 0; }
expand() { [ "${1:-all}" = all ] && echo "$DOGFOOD_REPOS" || echo "$1"; }

sync_to_dev() { # $1 worktree dir
  git -C "$1" rev-parse --git-dir >/dev/null 2>&1 || { echo "  ! $1 is not a git worktree"; return 1; }
  git -C "$1" fetch origin "$UPSTREAM_BRANCH" -q 2>/dev/null
  echo "  $1 -> $(git -C "$1" reset --hard "origin/$UPSTREAM_BRANCH" 2>&1 | tail -1)"
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

# Rebuild the BIN from substrate origin/dev if stale — used by start/restart/bin (mutating).
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
# depts create worktrees under FKST_RUNTIME_ROOT, registered in the shared .git; each
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

start_one() {
  cfg "$1" || return 1
  local existing; existing=$(pidof_df)
  if [ -n "$existing" ]; then echo "[$1] already running (pid $existing) — use restart"; return 0; fi
  local ts log rt; ts=$(date +%s); log="$LOGDIR/${1}-sv-${ts}.log"; rt="$LOGDIR/dogfood-rt-${1}.${ts}"
  clean_stale_runtime_worktrees "$1" "$rt"
  local roots=( --package-root "$PKGSRC/packages/github-devloop"
                --package-root "$PKGSRC/packages/github-proxy"
                --package-root "$PKGSRC/packages/consensus" )
  local e; for e in $EXTRA; do roots+=( --package-root "$e" ); done
  BIN="$BIN" FKST_GITHUB_REPO="$REPO" FKST_GITHUB_WRITE=1 FKST_GITHUB_BOT_LOGIN="$BOT" \
    FKST_DEVLOOP_UPSTREAM_BRANCH="$UPSTREAM_BRANCH" FKST_DEVLOOP_INTEGRATION_BRANCH="$INTEGRATION_BRANCH" \
    FKST_DEVLOOP_ROLLUP_MERGE="$ROLLUP_MERGE" \
    FKST_RUNTIME_ROOT="$rt" FKST_DURABLE_ROOT="$DUR" FKST_RATE_POOL_ROOT="$RATE_POOL" \
    nohup "$BIN" supervise --project-root "$HOST" "${roots[@]}" --framework-bin "$BIN" > "$log" 2>&1 &
  local pid=$!
  ln -sf "$log" "$LOGDIR/${1}-sv.log"
  sleep 3
  if kill -0 "$pid" 2>/dev/null; then
    echo "[$1] started pid $pid  panic=$(grep -ac panicked "$log" 2>/dev/null)  log=$log"
  else
    echo "[$1] FAILED to start; tail:"; tail -6 "$log" | sed 's/\x1b\[[0-9;]*m//g'
  fi
}

stop_one() {
  cfg "$1" || return 1
  local p; p=$(pidof_df)
  if [ -z "$p" ]; then echo "[$1] not running"; return 0; fi
  kill -9 $p 2>/dev/null; echo "[$1] killed $p"
}

restart_one() {
  cfg "$1" || return 1
  echo "[$1] sync to origin/$UPSTREAM_BRANCH:"
  sync_to_dev "$PKGSRC"
  [ "$HOST" != "$PKGSRC" ] && sync_to_dev "$HOST"
  stop_one "$1"; sleep 1
  start_one "$1"
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

doctor_one() {
  cfg "$1" || return 1
  local p log panic pdev sdev procpkg proceng pv ev; p=$(pidof_df); log=$(latest_log "$1")
  panic=$(grep -ac panicked "$log" 2>/dev/null); panic=${panic:-0}
  if [ -z "$p" ]; then printf '  %-9s STOPPED (target %s)\n' "$1" "$REPO"; return 0; fi
  git -C "$PKGSRC" fetch origin "$UPSTREAM_BRANCH" -q 2>/dev/null
  pdev=$(git -C "$PKGSRC" rev-parse "origin/$UPSTREAM_BRANCH" 2>/dev/null)
  sdev=$(git -C "$SUBSTRATE_SRC" rev-parse "origin/$UPSTREAM_BRANCH" 2>/dev/null)
  # Authoritative freshness = the code the RUNNING process loaded at startup (logged code_provenance
  # PKG_VERS/ENGINE_VER), NOT the worktree/BIN file — those can be updated without reloading the
  # process. A sync-without-restart leaves the process stale while the worktree looks current; only
  # the logged provenance catches it. Restart is the ONLY thing that reloads.
  procpkg=$(grep -aoE 'PKG_VERS=[^ ]*github-devloop@[a-f0-9]+' "$log" 2>/dev/null | grep -oE 'github-devloop@[a-f0-9]+' | tail -1 | cut -d@ -f2)
  proceng=$(grep -aoE 'ENGINE_VER=[a-f0-9]+' "$log" 2>/dev/null | tail -1 | cut -d= -f2)
  if   [ -z "$procpkg" ]; then pv="pkg=?"
  elif [ "${pdev:0:${#procpkg}}" = "$procpkg" ]; then pv="pkg-current"
  elif [ -z "$(git -C "$PKGSRC" diff "$procpkg" "$pdev" -- packages/ 2>/dev/null)" ]; then pv="pkg-skew(non-package, no restart)"
  else pv="PKG-STALE→restart(${procpkg:0:8}≠${pdev:0:8})"; fi
  if   [ -z "$proceng" ]; then ev="engine=?"
  elif [ "${sdev:0:${#proceng}}" = "$proceng" ]; then ev="engine-current"
  else ev="ENGINE-STALE→restart(${proceng:0:8}≠${sdev:0:8})"; fi
  printf '  %-9s RUNNING pid %s up %s | %s %s | worktree %s | panic %s\n' "$1" "$p" \
    "$(ps -o etime= -p $p 2>/dev/null|tr -d ' ')" "$pv" "$ev" "$(git -C "$PKGSRC" rev-parse --short HEAD 2>/dev/null)" "$panic"
}

cmd_doctor() {
  echo "engine BIN:"; bin_freshness_report | sed 's/^/  /'
  echo "supervises:"
  for n in $(expand "${1:-all}"); do doctor_one "$n"; done
  echo "graphql: $(gh api rate_limit --jq '.resources.graphql.remaining' 2>/dev/null||echo ?)/5000"
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
  gh api "repos/$REPO/issues?state=open&per_page=100" --jq '.[]|select(.pull_request==null)|"\(.number)\t\(.updated_at)\t\([.labels[].name]|map(select(startswith("fkst-dev:")and .!="fkst-dev:enabled"))|join(","))\t\(.title[0:38])"' 2>/dev/null | \
  while IFS=$'\t' read -r num upd label title; do
    [ -z "$label" ] && continue
    local a st cls; a=$(( (now - $(epoch_utc "$upd")) / 3600 )); st="${label#fkst-dev:}"; st="${st%%,*}"
    case "$st" in
      tracking|pr-open) cls="tracking/umbrella" ;;
      blocked|impl-failed|merged|declined) cls="parked($st)" ;;
      thinking|ready|implementing|stalled-thinking) [ "$a" -ge "$stale" ] && cls="⚠ STUCK $st ${a}h" || cls="✓ flowing $st ${a}h" ;;
      reviewing|fixing|review-meta|merge-ready|merging)
        if echo "$openpr"|grep -qx "$num"; then cls="$st →see PR (active)"; else cls="⚠ STRANDED $st (no open PR)"; fi ;;
      *) continue ;;
    esac
    printf "  #%-4s [%-12s] %s\n" "$num" "$st" "$cls"
  done
  echo ""
}

cmd_config() {
  echo "resolved config ($([ -f "$_cfg" ] && echo "from $_cfg" || echo 'defaults only — no per-machine config file'))"
  printf '  %-18s %s\n' DOGFOOD_ROOT "$DOGFOOD_ROOT" SUBSTRATE_SRC "$SUBSTRATE_SRC" BIN "$BIN" \
    BOT "$BOT" GH_ORG "$GH_ORG" UPSTREAM_BRANCH "$UPSTREAM_BRANCH" INTEGRATION_BRANCH "$INTEGRATION_BRANCH" \
    ROLLUP_MERGE "$ROLLUP_MERGE" RATE_POOL "$RATE_POOL" LOGDIR "$LOGDIR" DOGFOOD_REPOS "$DOGFOOD_REPOS"
  echo "per-repo (HOST | PKGSRC | DURABLE):"
  local n; for n in $DOGFOOD_REPOS; do cfg "$n" && printf '  %-9s %s | %s | %s\n' "$n" "$HOST" "$PKGSRC" "$DUR"; done
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
  start)   bin_ensure_fresh; for n in $(expand "${arg2:-all}"); do start_one   "$n"; done ;;
  stop)    for n in $(expand "${arg2:-all}"); do stop_one "$n"; done ;;
  restart) bin_ensure_fresh; for n in $(expand "${arg2:-all}"); do restart_one "$n"; done ;;
  status)  for n in $(expand "${arg2:-all}"); do status_one "$n"; done ;;
  doctor)  cmd_doctor "${arg2:-all}" ;;
  config)  cmd_config ;;
  board)   cmd_board "$arg2" "$arg3" ;;
  logs)    f=$(latest_log "${arg2:-packages}"); echo "$f"; tail -"${arg3:-40}" "$f" 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' ;;
  *) echo "usage: $0 {status|doctor|config|board|bin|start|stop|restart|logs} [packages|substrate|website|all] [stale_h|lines]"; exit 1 ;;
esac
