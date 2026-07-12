# Machine fkst Home Layout

Use `~/.fkst` as the single authoritative user-scope root for all machine-local
fkst state. A single root makes the installation discoverable, backupable, and
relocatable without mixing fkst state with unrelated home-directory content.

This convention extends two existing precedents rather than introducing a new
resolver:

- fkst-substrate documents `~/.fkst/rate-pools` as the default
  `FKST_RATE_POOL_ROOT`.
- The [workflow orchestration design](../superpowers/specs/2026-07-02-workflow-orchestration-layer-design.md)
  assigns user-authored workflow contracts to `~/.fkst/workflow/`.

## Canonical Layout

```text
~/.fkst/
  rate-pools/                 # Engine rate-pool ledgers; existing contract
  workflow/                   # User-authored workflow contracts; existing spec
  dogfood/                    # DOGFOOD_ROOT: run checkouts, durable/, logs, and runtime scratch
  src/fkst-packages/          # Operator source checkout; pinned dev mirror
  src/fkst-substrate/         # Operator engine checkout and BIN build root
  worktrees/<repo>/<topic>/   # Future work worktrees
```

The `worktrees/` location applies to newly created worktrees. Do not relocate
stale legacy worktrees into it. Preserve each unique tip and then delete each
stale worktree individually under the proof rules below.

## Migration Context

The machine motivating this migration currently has three live supervises under
`~/.fkst-dogfood`, a concurrent long-running agent session with registered Git
worktrees under that tree, and an operator session whose current directory is
inside `~/fkst-packages`. Those are hard constraints: moving a live tree would
break registered paths and process current directories, while replacing a
durable root would strand in-flight delivery state.

The migration is phased. A later phase must not begin until every command in the
earlier phase exits zero and produces the stated result. Any other result is an
abort unless that phase defines an explicit rollback.

## P0: Publish the Convention (Now)

This PR intentionally changes exactly these three repository paths:
`docs/user/machine-fkst-home-layout.md`,
`.claude/skills/dogfood-github-devloop/dogfood.config.example.sh`, and
`.claude/skills/dogfood-github-devloop/SKILL.md`. Harness scratch artifacts in
the working tree are not part of the change set.

Verify the published references mechanically. Expected result: every command
exits zero and prints a match; otherwise abort the PR.

```sh
set -eu
rg -n 'machine-fkst-home-layout\.md' \
  .claude/skills/dogfood-github-devloop/SKILL.md \
  .claude/skills/dogfood-github-devloop/dogfood.config.example.sh
rg -n '\.fkst/(dogfood|src/fkst-substrate)' \
  .claude/skills/dogfood-github-devloop/dogfood.config.example.sh
```

The built-in defaults in `dogfood.sh` intentionally remain on the legacy paths
until P2 activation. Changing a script default affects every machine, while the
gitignored `dogfood.config.sh` files on other machines cannot be verified from
this repository. Flipping those defaults is a follow-up PR only after every
known machine has pinned its legacy paths or migrated.

## P1: Remove Proven-Stale Duplicate Checkouts (Now, Per Item)

Run the following block independently for each standalone duplicate checkout.
Set `IGNORED_POLICY=archive` to preserve ignored files, or set
`IGNORED_POLICY=accept-loss` and `ACCEPT_IGNORED_LOSS=yes` to state explicitly
that ignored files will be lost. A Git bundle preserves commits only; it does
not preserve dirty, untracked, ignored, or stashed state.

Expected result: the checkout is clean, the shared stash is empty, ignored
files are either archived or explicitly accepted-lost, no process holds the
tree, and every local branch is reachable from a remote ref or a verified
bundle. Any failed assertion aborts deletion.

```sh
set -eu
CHECKOUT=/absolute/path/to/duplicate
BACKUP_DIR="$HOME/.fkst/backups"
IGNORED_POLICY=archive                 # archive | accept-loss
ACCEPT_IGNORED_LOSS=no                # set yes only with accept-loss

git -C "$CHECKOUT" fetch --all --prune
test -d "$CHECKOUT/.git"              # abort: this is a linked worktree
test -z "$(git -C "$CHECKOUT" status --porcelain)"
test -z "$(git -C "$CHECKOUT" stash list)"

mkdir -p "$BACKUP_DIR"
ignored_list=$(mktemp)
git -C "$CHECKOUT" ls-files --others --ignored --exclude-standard -z >"$ignored_list"
if test -s "$ignored_list"; then
  case "$IGNORED_POLICY" in
    archive)
      ignored_tar="$BACKUP_DIR/$(basename "$CHECKOUT")-ignored-$(date +%Y%m%dT%H%M%S).tar"
      tar -C "$CHECKOUT" --null -T "$ignored_list" -cf "$ignored_tar"
      tar -tf "$ignored_tar" >/dev/null
      ;;
    accept-loss)
      test "$ACCEPT_IGNORED_LOSS" = yes
      ;;
    *) exit 1 ;;
  esac
fi
rm -f "$ignored_list"

held=$(mktemp)
lsof +D "$CHECKOUT" >"$held" 2>/dev/null || true
test ! -s "$held"
rm -f "$held"

unreachable=0
while IFS= read -r ref; do
  if test -z "$(git -C "$CHECKOUT" for-each-ref --contains="$ref" --format='%(refname)' refs/remotes)"; then
    printf 'UNREACHABLE %s %s\n' "$ref" "$(git -C "$CHECKOUT" rev-parse "$ref")"
    unreachable=1
  fi
done <<EOF
$(git -C "$CHECKOUT" for-each-ref --format='%(refname)' refs/heads)
EOF

if test "$unreachable" -ne 0; then
  bundle="$BACKUP_DIR/$(basename "$CHECKOUT")-$(date +%Y%m%dT%H%M%S).bundle"
  git -C "$CHECKOUT" bundle create "$bundle" --branches
  git bundle verify "$bundle"
fi
```

Only after the block exits zero may the operator remove that one checkout.

## P2: Move the Dogfood Root (Deferred)

Use an approved stop window and run every P2 block below in the same shell
session so its checked variables, functions, and snapshots remain available.
Do not transcribe paths from memory. The first block captures the
precedence-resolved `dogfood.sh config` output and derives every asserted old and
new path from it. On the reference machine, the resolved durable paths are
`$DOGFOOD_ROOT/durable/packages`, `$DOGFOOD_ROOT/durable/substrate`, and
`$DOGFOOD_ROOT/durable/website`; those are the real names used by the examples.
Each new path is the old resolved path relocated with the complete tree, never a
fresh replacement. In this phase `SUBSTRATE_SRC` and `BIN` remain at their
resolved pre-P2 paths; P3 moves them.

The preflight below inventories every repository under the old dogfood root,
records each worktree registration, proves the destination absent, and saves the
machine config. It then stops every target and proves `lsof +D` is empty before
snapshotting durable counts. This is the stable baseline: no supervise may
change the stores while it is recorded. If stop or quiescence fails, the block
starts every target again and requires a green doctor before aborting, so a
pre-move failure never strands stopped targets.

```sh
set -eu
DOGFOOD="$HOME/fkst-packages/.claude/skills/dogfood-github-devloop/dogfood.sh"
CONFIG="$HOME/fkst-packages/.claude/skills/dogfood-github-devloop/dogfood.config.sh"
NEW_ROOT="$HOME/.fkst/dogfood"

test -x "$DOGFOOD"
test -f "$CONFIG"
mkdir -p "$HOME/.fkst" "$HOME/.fkst/backups"

config_before=$(mktemp)
"$DOGFOOD" config >"$config_before"
cat "$config_before"
resolved_from() { awk -v key="$2" '$1 == key {print $2}' "$1"; }
resolved_dur_from() {
  awk -F'\\|' -v name="$2" '
    $1 ~ "^[[:space:]]*" name "[[:space:]]" {
      value=$3; gsub(/^[[:space:]]+|[[:space:]]+$/, "", value); print value
    }' "$1"
}
relocate_from_root() {
  case "$1/" in
    "$OLD_ROOT"/*) printf '%s\n' "$NEW_ROOT${1#"$OLD_ROOT"}" ;;
    *) printf '%s\n' "$1" ;;
  esac
}
assert_under() { case "$1/" in "$2"/*) ;; *) return 1 ;; esac; }

OLD_ROOT=$(resolved_from "$config_before" DOGFOOD_ROOT)
OLD_DUR_PACKAGES=$(resolved_dur_from "$config_before" packages)
OLD_DUR_SUBSTRATE=$(resolved_dur_from "$config_before" substrate)
OLD_DUR_WEBSITE=$(resolved_dur_from "$config_before" website)
OLD_RATE_POOL=$(resolved_from "$config_before" RATE_POOL)
OLD_LOGDIR=$(resolved_from "$config_before" LOGDIR)
EXPECTED_SUBSTRATE_SRC=$(resolved_from "$config_before" SUBSTRATE_SRC)
EXPECTED_BIN=$(resolved_from "$config_before" BIN)
EXPECTED_GH_ORG=$(resolved_from "$config_before" GH_ORG)
NEW_DUR_PACKAGES=$(relocate_from_root "$OLD_DUR_PACKAGES")
NEW_DUR_SUBSTRATE=$(relocate_from_root "$OLD_DUR_SUBSTRATE")
NEW_DUR_WEBSITE=$(relocate_from_root "$OLD_DUR_WEBSITE")
EXPECTED_RATE_POOL=$(relocate_from_root "$OLD_RATE_POOL")
EXPECTED_LOGDIR=$(relocate_from_root "$OLD_LOGDIR")
rm -f "$config_before"

test -n "$OLD_ROOT"
test -n "$OLD_DUR_PACKAGES"
test -n "$OLD_DUR_SUBSTRATE"
test -n "$OLD_DUR_WEBSITE"
test -n "$EXPECTED_SUBSTRATE_SRC"
test -x "$EXPECTED_BIN"
test -d "$OLD_ROOT"
test ! -e "$NEW_ROOT"
test "$OLD_DUR_PACKAGES" = "$OLD_ROOT/durable/packages"
test "$OLD_DUR_SUBSTRATE" = "$OLD_ROOT/durable/substrate"
test "$OLD_DUR_WEBSITE" = "$OLD_ROOT/durable/website"

assert_doctor_green() {
  doctor_out=$(mktemp)
  if ! "$DOGFOOD" doctor all >"$doctor_out"; then
    cat "$doctor_out"; rm -f "$doctor_out"; return 1
  fi
  cat "$doctor_out"
  failed=0
  for target in packages substrate website; do
    grep -E "^[[:space:]]+$target[[:space:]]+RUNNING .*pkg-current engine-current" \
      "$doctor_out" >/dev/null || failed=1
  done
  grep -E '^[[:space:]].*(STOPPED|PKG-STALE|ENGINE-STALE|CONFIG-ERROR|STRAY)' \
    "$doctor_out" >/dev/null && failed=1
  rm -f "$doctor_out"
  test "$failed" -eq 0
}

assert_board_green() {
  board_out=$(mktemp)
  if ! "$DOGFOOD" board all >"$board_out"; then
    cat "$board_out"; rm -f "$board_out"; return 1
  fi
  cat "$board_out"
  failed=0
  for repo in fkst-packages fkst-substrate fkst-website; do
    header="════════════════════════════════════════ $EXPECTED_GH_ORG/$repo"
    grep -Fx -- "$header" "$board_out" >/dev/null || failed=1
    awk -v header="$header" '
      $0 == header {inside=1; next}
      inside && /^═/ {exit}
      inside && /^supervise: pid [0-9]+ up / {found=1}
      END {exit(found ? 0 : 1)}' "$board_out" || failed=1
  done
  grep -E '^[[:space:]]+(PR#|#).*(STUCK|STRANDED|CI-RED|NO-CI)' \
    "$board_out" >/dev/null && failed=1
  rm -f "$board_out"
  test "$failed" -eq 0
}

snapshot_durable_counts() {
  output=$1; shift
  raw=$(mktemp)
  : >"$raw"
  for row in "$@"; do
    name=${row%%:*}; dur=${row#*:}
    test -f "$dur/delivery.redb" || { rm -f "$raw"; return 1; }
    "$EXPECTED_BIN" observe --json --durable-root "$dur" |
      jq -er --arg name "$name" '
        [$name, (([.queues[].pending] | add) // 0), (.dead_letters | length)] | @tsv' \
      >>"$raw" || { rm -f "$raw"; return 1; }
  done
  sort "$raw" >"$output"
  rm -f "$raw"
  test "$(wc -l <"$output" | tr -d ' ')" -eq 3
}

repo_candidates=$(mktemp)
repo_roots=$(mktemp)
repo_list=$(mktemp)
find "$OLD_ROOT" -name .git -print >"$repo_candidates"
: >"$repo_roots"
while IFS= read -r dotgit; do
  worktree_list=$(mktemp)
  git -C "${dotgit%/.git}" -c core.quotePath=false worktree list --porcelain \
    >"$worktree_list"
  awk '/^worktree / {print substr($0,10); exit}' "$worktree_list" >>"$repo_roots"
  rm -f "$worktree_list"
done <"$repo_candidates"
sort -u "$repo_roots" >"$repo_list"
rm -f "$repo_candidates" "$repo_roots"
test -s "$repo_list"
worktree_inventory="$HOME/.fkst/backups/dogfood-worktrees-before.tsv"
: >"$worktree_inventory"
while IFS= read -r repo; do
  worktree_list=$(mktemp)
  git -C "$repo" -c core.quotePath=false worktree list --porcelain \
    >"$worktree_list"
  awk -v repo="$repo" '/^worktree / {print repo "\t" substr($0,10)}' \
    "$worktree_list" >>"$worktree_inventory"
  rm -f "$worktree_list"
done <"$repo_list"
test -s "$worktree_inventory"

config_backup="$HOME/.fkst/backups/dogfood.config.before-p2.sh"
cp "$CONFIG" "$config_backup"
rollback_config_p2="$HOME/.fkst/backups/dogfood.config.rollback-p2.sh"
{
  printf '. "%s"\n' "$config_backup"
  printf 'DOGFOOD_ROOT="%s"\n' "$NEW_ROOT"
  printf 'DUR_PACKAGES="%s"\n' "$NEW_DUR_PACKAGES"
  printf 'DUR_SUBSTRATE="%s"\n' "$NEW_DUR_SUBSTRATE"
  printf 'DUR_WEBSITE="%s"\n' "$NEW_DUR_WEBSITE"
  printf 'RATE_POOL="%s"\n' "$EXPECTED_RATE_POOL"
  printf 'LOGDIR="%s"\n' "$EXPECTED_LOGDIR"
  printf 'SUBSTRATE_SRC="%s"\n' "$EXPECTED_SUBSTRATE_SRC"
  printf 'BIN="%s"\n' "$EXPECTED_BIN"
} >"$rollback_config_p2"

recover_pre_move() {
  recovery_failed=0
  "$DOGFOOD" start all || recovery_failed=1
  assert_doctor_green || recovery_failed=1
  test "$recovery_failed" -eq 0
}

if ! "$DOGFOOD" stop all; then
  recover_pre_move
  exit 1
fi
held=$(mktemp)
lsof +D "$OLD_ROOT" >"$held" 2>/dev/null || true
if test -s "$held"; then
  cat "$held"
  rm -f "$held"
  recover_pre_move
  exit 1
fi
rm -f "$held"

counts_before="$HOME/.fkst/backups/dogfood-durable-before.tsv"
if ! snapshot_durable_counts "$counts_before" \
    "packages:$OLD_DUR_PACKAGES" \
    "substrate:$OLD_DUR_SUBSTRATE" \
    "website:$OLD_DUR_WEBSITE"; then
  recover_pre_move
  exit 1
fi
```

Define rollback before moving. Rollback is intentionally fail-closed: if
`stop all` fails, it does not touch the live tree. It also requires `lsof +D` to
be empty on `NEW_ROOT`, moves the tree back, restores the config, and repairs
every inventory entry from its restored old repository and worktree path. Git
worktree repair is directional; this second repair reverses the earlier one.
Successful rollback ends with `start all`, positive doctor and board checks, and
the durable-count sanity check. If rollback itself aborts before the move, stop
and resolve the live holder rather than forcing the tree.

```sh
set -eu
p2_rollback() {
  if ! DOGFOOD_CONFIG="$rollback_config_p2" "$DOGFOOD" stop all; then
    echo "ROLLBACK: stop all failed; retrying once" >&2
    if ! DOGFOOD_CONFIG="$rollback_config_p2" "$DOGFOOD" stop all; then
      recovery_failed=0
      DOGFOOD_CONFIG="$rollback_config_p2" "$DOGFOOD" start all || recovery_failed=1
      DOGFOOD_CONFIG="$rollback_config_p2" assert_doctor_green || recovery_failed=1
      if test -d "$NEW_ROOT" && test ! -e "$OLD_ROOT"; then
        tree_state="NEW_ROOT exists and OLD_ROOT is absent; the tree remains moved"
      elif test -d "$OLD_ROOT" && test ! -e "$NEW_ROOT"; then
        tree_state="OLD_ROOT exists and NEW_ROOT is absent; the tree remains at its original location"
      else
        tree_state="ambiguous; inspect OLD_ROOT and NEW_ROOT before taking action"
      fi
      echo "ESCALATE MANUALLY: stop all failed twice; rollback did NOT move the tree; state is: $tree_state" >&2
      test "$recovery_failed" -eq 0 || \
        echo "ESCALATE MANUALLY: start all or RUNNING doctor verification also failed" >&2
      return 1
    fi
  fi
  held=$(mktemp)
  lsof +D "$NEW_ROOT" >"$held" 2>/dev/null || true
  if test -s "$held"; then
    cat "$held"
    rm -f "$held"
    echo "ROLLBACK ABORTED: NEW_ROOT is still held; the tree was not moved" >&2
    return 1
  fi
  rm -f "$held"

  test -d "$NEW_ROOT"
  test ! -e "$OLD_ROOT"
  mv "$NEW_ROOT" "$OLD_ROOT"
  cp "$config_backup" "$CONFIG"

  while IFS="$(printf '\t')" read -r old_repo old_wt; do
    test -d "$old_repo"
    test -e "$old_wt"
    git -C "$old_repo" worktree repair "$old_wt"
    git -C "$old_repo" -c core.quotePath=false worktree list --porcelain |
      awk '/^worktree / {print substr($0,10)}' | grep -Fx -- "$old_wt"
  done <"$worktree_inventory"

  "$DOGFOOD" start all
  assert_doctor_green
  assert_board_green
  rollback_counts=$(mktemp)
  snapshot_durable_counts "$rollback_counts" \
    "packages:$OLD_DUR_PACKAGES" \
    "substrate:$OLD_DUR_SUBSTRATE" \
    "website:$OLD_DUR_WEBSITE"
  cmp "$counts_before" "$rollback_counts"
  rm -f "$rollback_counts"
}
```

Move the complete tree, edit the gitignored config, assert the newly resolved
paths, repair every registration, sync, and explicitly start the stopped
targets. `sync` does not start a stopped target. Every failure after the first
`mv`, including edit, assertion, repair, sync, start, doctor, board, or durable
verification, invokes the rollback above.

The doctor and board gates are positive. Doctor must contain one current
`RUNNING` line per target. Board must contain the expected repository section
header and a `supervise: pid ... up ...` line inside every section. Empty output,
a missing section, or a suppressed board API failure therefore fails the gate.

The whole tree is preserved by one same-filesystem `mv`; there is no copy or
file-by-file transformation. The exact pending/dead-letter comparison after
restart is only a sanity check that the restarted processes reopened the same
quiescent stores. It is not byte-level proof of durable contents, and a mismatch
must be investigated rather than described as evidence that `mv` lost bytes.

```sh
set -eu
set +e
(
  set -eu
  mv "$OLD_ROOT" "$NEW_ROOT"
  ${EDITOR:?set EDITOR} "$CONFIG"

  config_out=$(mktemp)
  "$DOGFOOD" config >"$config_out"
  cat "$config_out"
  test "$(resolved_from "$config_out" DOGFOOD_ROOT)" = "$NEW_ROOT"
  test "$(resolved_dur_from "$config_out" packages)" = "$NEW_DUR_PACKAGES"
  test "$(resolved_dur_from "$config_out" substrate)" = "$NEW_DUR_SUBSTRATE"
  test "$(resolved_dur_from "$config_out" website)" = "$NEW_DUR_WEBSITE"
  test "$(resolved_from "$config_out" RATE_POOL)" = "$EXPECTED_RATE_POOL"
  test "$(resolved_from "$config_out" LOGDIR)" = "$EXPECTED_LOGDIR"
  test "$(resolved_from "$config_out" SUBSTRATE_SRC)" = "$EXPECTED_SUBSTRATE_SRC"
  test "$(resolved_from "$config_out" BIN)" = "$EXPECTED_BIN"
  assert_under "$(resolved_from "$config_out" DOGFOOD_ROOT)" "$HOME/.fkst"
  assert_under "$(resolved_dur_from "$config_out" packages)" "$HOME/.fkst"
  assert_under "$(resolved_dur_from "$config_out" substrate)" "$HOME/.fkst"
  assert_under "$(resolved_dur_from "$config_out" website)" "$HOME/.fkst"
  assert_under "$(resolved_from "$config_out" RATE_POOL)" "$HOME/.fkst"
  assert_under "$(resolved_from "$config_out" LOGDIR)" "$HOME/.fkst"
  assert_under "$(resolved_from "$config_out" BIN)" "$EXPECTED_SUBSTRATE_SRC"
  rm -f "$config_out"

  while IFS="$(printf '\t')" read -r old_repo old_wt; do
    new_repo=$(relocate_from_root "$old_repo")
    new_wt=$(relocate_from_root "$old_wt")
    test -d "$new_repo"
    if test -e "$new_wt"; then
      repaired_wt=$new_wt
    else
      test -e "$old_wt"
      repaired_wt=$old_wt
    fi
    git -C "$new_repo" worktree repair "$repaired_wt"
    git -C "$new_repo" -c core.quotePath=false worktree list --porcelain |
      awk '/^worktree / {print substr($0,10)}' | grep -Fx -- "$repaired_wt"
  done <"$worktree_inventory"

  "$DOGFOOD" sync all
  "$DOGFOOD" start all
  assert_doctor_green
  assert_board_green

  counts_after=$(mktemp)
  snapshot_durable_counts "$counts_after" \
    "packages:$NEW_DUR_PACKAGES" \
    "substrate:$NEW_DUR_SUBSTRATE" \
    "website:$NEW_DUR_WEBSITE"
  cmp "$counts_before" "$counts_after"
  rm -f "$counts_after"
)
move_status=$?
set -e
if test "$move_status" -ne 0; then
  p2_rollback
  exit 1
fi
```

Only after P2 validation passes should a follow-up PR change the `dogfood.sh`
built-in defaults. Before that PR merges, verify every known machine has either
migrated or explicitly pinned its current paths.

## P3: Move Operator Source Checkouts (Deferred)

P3 covers every surviving linked worktree of both operator checkouts, not just
the two main directories. Before taking the repair inventory, enumerate the raw
porcelain registrations. A missing/prunable worktree may be the only name for a
detached or unpushed commit, so preserve each unique such tip in
`~/.fkst/backups/` and verify its bundle before pruning stale registrations.
Only then take the inventory used by post-move repair.

`dogfood.sh` lives below `OLD_PACKAGES`, and every child resolves `BIN` below
`OLD_SUBSTRATE` when it spawns. P3 therefore stops all targets before either
checkout moves. After stop, every inventoried tree must exist and every
`lsof +D` result must be empty. A pre-move failure starts all targets again and
requires a green doctor before aborting.

```sh
set -eu
DOGFOOD="$HOME/fkst-packages/.claude/skills/dogfood-github-devloop/dogfood.sh"
CONFIG="$HOME/fkst-packages/.claude/skills/dogfood-github-devloop/dogfood.config.sh"
OLD_PACKAGES="$HOME/fkst-packages"
OLD_SUBSTRATE="$HOME/fkst-substrate"
NEW_PACKAGES="$HOME/.fkst/src/fkst-packages"
NEW_SUBSTRATE="$HOME/.fkst/src/fkst-substrate"
mkdir -p "$HOME/.fkst/src" "$HOME/.fkst/backups"
test ! -e "$NEW_PACKAGES"
test ! -e "$NEW_SUBSTRATE"

config_before_p3=$(mktemp)
"$DOGFOOD" config >"$config_before_p3"
cat "$config_before_p3"
resolved_from() { awk -v key="$2" '$1 == key {print $2}' "$1"; }
resolved_dur_from() {
  awk -F'\\|' -v name="$2" '
    $1 ~ "^[[:space:]]*" name "[[:space:]]" {
      value=$3; gsub(/^[[:space:]]+|[[:space:]]+$/, "", value); print value
    }' "$1"
}
assert_under() { case "$1/" in "$2"/*) ;; *) return 1 ;; esac; }
OLD_DOGFOOD_ROOT=$(resolved_from "$config_before_p3" DOGFOOD_ROOT)
OLD_DUR_PACKAGES=$(resolved_dur_from "$config_before_p3" packages)
OLD_DUR_SUBSTRATE=$(resolved_dur_from "$config_before_p3" substrate)
OLD_DUR_WEBSITE=$(resolved_dur_from "$config_before_p3" website)
OLD_RATE_POOL=$(resolved_from "$config_before_p3" RATE_POOL)
OLD_LOGDIR=$(resolved_from "$config_before_p3" LOGDIR)
OLD_SUBSTRATE_SRC=$(resolved_from "$config_before_p3" SUBSTRATE_SRC)
OLD_BIN=$(resolved_from "$config_before_p3" BIN)
EXPECTED_GH_ORG=$(resolved_from "$config_before_p3" GH_ORG)
rm -f "$config_before_p3"
test "$OLD_SUBSTRATE_SRC" = "$OLD_SUBSTRATE"
test -x "$OLD_BIN"

assert_doctor_green() {
  doctor_out=$(mktemp)
  if ! "$DOGFOOD" doctor all >"$doctor_out"; then
    cat "$doctor_out"; rm -f "$doctor_out"; return 1
  fi
  cat "$doctor_out"
  failed=0
  for target in packages substrate website; do
    grep -E "^[[:space:]]+$target[[:space:]]+RUNNING .*pkg-current engine-current" \
      "$doctor_out" >/dev/null || failed=1
  done
  grep -E '^[[:space:]].*(STOPPED|PKG-STALE|ENGINE-STALE|CONFIG-ERROR|STRAY)' \
    "$doctor_out" >/dev/null && failed=1
  rm -f "$doctor_out"
  test "$failed" -eq 0
}

assert_board_green() {
  board_out=$(mktemp)
  if ! "$DOGFOOD" board all >"$board_out"; then
    cat "$board_out"; rm -f "$board_out"; return 1
  fi
  cat "$board_out"
  failed=0
  for repo in fkst-packages fkst-substrate fkst-website; do
    header="════════════════════════════════════════ $EXPECTED_GH_ORG/$repo"
    grep -Fx -- "$header" "$board_out" >/dev/null || failed=1
    awk -v header="$header" '
      $0 == header {inside=1; next}
      inside && /^═/ {exit}
      inside && /^supervise: pid [0-9]+ up / {found=1}
      END {exit(found ? 0 : 1)}' "$board_out" || failed=1
  done
  grep -E '^[[:space:]]+(PR#|#).*(STUCK|STRANDED|CI-RED|NO-CI)' \
    "$board_out" >/dev/null && failed=1
  rm -f "$board_out"
  test "$failed" -eq 0
}

snapshot_durable_counts() {
  output=$1; bin=$2; shift 2
  raw=$(mktemp)
  : >"$raw"
  for row in "$@"; do
    name=${row%%:*}; dur=${row#*:}
    test -f "$dur/delivery.redb" || { rm -f "$raw"; return 1; }
    "$bin" observe --json --durable-root "$dur" |
      jq -er --arg name "$name" '
        [$name, (([.queues[].pending] | add) // 0), (.dead_letters | length)] | @tsv' \
      >>"$raw" || { rm -f "$raw"; return 1; }
  done
  sort "$raw" >"$output"
  rm -f "$raw"
  test "$(wc -l <"$output" | tr -d ' ')" -eq 3
}

for repo in packages substrate; do
  eval "root=\$OLD_$(printf '%s' "$repo" | tr '[:lower:]' '[:upper:]')"
  raw_inventory="$HOME/.fkst/backups/p3-$repo-worktrees-raw.txt"
  records="$HOME/.fkst/backups/p3-$repo-worktree-records.tsv"
  seen_tips=$(mktemp)
  inventory="$HOME/.fkst/backups/p3-$repo-worktrees-before.txt"

  git -C "$root" fetch --all --prune
  git -C "$root" -c core.quotePath=false worktree list --porcelain >"$raw_inventory"
  awk '
    function emit() {
      if (wt != "") print wt "\t" head "\t" detached "\t" prunable
      wt=""; head=""; detached=0; prunable=0
    }
    /^worktree / {emit(); wt=substr($0,10); next}
    /^HEAD / {head=substr($0,6); next}
    /^detached$/ {detached=1; next}
    /^prunable / {prunable=1; next}
    /^$/ {emit()}
    END {emit()}' "$raw_inventory" >"$records"
  test -s "$records"

  while IFS="$(printf '\t')" read -r wt head detached prunable; do
    test -n "$head"
    remote_refs=$(git -C "$root" for-each-ref --contains="$head" \
      --format='%(refname)' refs/remotes)
    if test "$detached" = 1 || test -z "$remote_refs"; then
      if ! grep -Fx -- "$head" "$seen_tips" >/dev/null 2>&1; then
        backup_ref="refs/fkst-migration-backup/p3-$repo-$head"
        bundle="$HOME/.fkst/backups/p3-$repo-${head}-$(date +%Y%m%dT%H%M%S).bundle"
        git -C "$root" update-ref "$backup_ref" "$head"
        if ! git -C "$root" bundle create "$bundle" "$backup_ref"; then
          git -C "$root" update-ref -d "$backup_ref"
          exit 1
        fi
        if ! git -C "$root" bundle verify "$bundle"; then
          git -C "$root" update-ref -d "$backup_ref"
          exit 1
        fi
        git -C "$root" update-ref -d "$backup_ref"
        printf '%s\n' "$head" >>"$seen_tips"
      fi
    fi
  done <"$records"
  rm -f "$seen_tips"

  git -C "$root" worktree prune
  pruned_worktree_list=$(mktemp)
  git -C "$root" -c core.quotePath=false worktree list --porcelain \
    >"$pruned_worktree_list"
  awk '/^worktree / {print substr($0,10)}' "$pruned_worktree_list" >"$inventory"
  rm -f "$pruned_worktree_list"
  test -s "$inventory"
done

config_backup_p3="$HOME/.fkst/backups/dogfood.config.before-p3.sh"
cp "$CONFIG" "$config_backup_p3"
rollback_config_p3="$HOME/.fkst/backups/dogfood.config.rollback-p3.sh"
case "$OLD_BIN/" in
  "$OLD_SUBSTRATE"/*) ROLLBACK_BIN="$NEW_SUBSTRATE${OLD_BIN#"$OLD_SUBSTRATE"}" ;;
  *) ROLLBACK_BIN=$OLD_BIN ;;
esac
{
  printf '. "%s"\n' "$config_backup_p3"
  printf 'SUBSTRATE_SRC="%s"\n' "$NEW_SUBSTRATE"
  printf 'BIN="%s"\n' "$ROLLBACK_BIN"
} >"$rollback_config_p3"

recover_p3_pre_move() {
  recovery_failed=0
  "$DOGFOOD" start all || recovery_failed=1
  assert_doctor_green || recovery_failed=1
  test "$recovery_failed" -eq 0
}

if ! "$DOGFOOD" stop all; then
  recover_p3_pre_move
  exit 1
fi

for repo in packages substrate; do
  inventory="$HOME/.fkst/backups/p3-$repo-worktrees-before.txt"
  while IFS= read -r wt; do
    test -e "$wt" || { recover_p3_pre_move; exit 1; }
    held=$(mktemp)
    lsof +D "$wt" >"$held" 2>/dev/null || true
    if test -s "$held"; then
      cat "$held"; rm -f "$held"; recover_p3_pre_move; exit 1
    fi
    rm -f "$held"
  done <"$inventory"
done

p3_counts_before="$HOME/.fkst/backups/p3-durable-before.tsv"
if ! snapshot_durable_counts "$p3_counts_before" "$OLD_BIN" \
    "packages:$OLD_DUR_PACKAGES" \
    "substrate:$OLD_DUR_SUBSTRATE" \
    "website:$OLD_DUR_WEBSITE"; then
  recover_p3_pre_move
  exit 1
fi
```

Define rollback before either move. It discovers the usable dogfood script on
whichever side of a partial move exists, stops all targets, proves every moved
new tree is unheld, moves each completed checkout back, restores the old config,
and repairs all surviving registrations from the restored old roots. It then
starts all targets and runs positive doctor, board, and durable verification.

```sh
set -eu
current_dogfood() {
  if test -x "$NEW_PACKAGES/.claude/skills/dogfood-github-devloop/dogfood.sh"; then
    printf '%s\n' "$NEW_PACKAGES/.claude/skills/dogfood-github-devloop/dogfood.sh"
  else
    printf '%s\n' "$OLD_PACKAGES/.claude/skills/dogfood-github-devloop/dogfood.sh"
  fi
}

p3_rollback() {
  rollback_dogfood=$(current_dogfood)
  if ! DOGFOOD_CONFIG="$rollback_config_p3" "$rollback_dogfood" stop all; then
    echo "ROLLBACK: stop all failed; retrying once" >&2
    if ! DOGFOOD_CONFIG="$rollback_config_p3" "$rollback_dogfood" stop all; then
      recovery_failed=0
      DOGFOOD_CONFIG="$rollback_config_p3" "$rollback_dogfood" start all || recovery_failed=1
      DOGFOOD="$rollback_dogfood" DOGFOOD_CONFIG="$rollback_config_p3" \
        assert_doctor_green || recovery_failed=1
      if test -d "$NEW_PACKAGES" && test ! -e "$OLD_PACKAGES"; then
        packages_state="moved"
      elif test -d "$OLD_PACKAGES" && test ! -e "$NEW_PACKAGES"; then
        packages_state="not moved"
      else
        packages_state="ambiguous"
      fi
      if test -d "$NEW_SUBSTRATE" && test ! -e "$OLD_SUBSTRATE"; then
        substrate_state="moved"
      elif test -d "$OLD_SUBSTRATE" && test ! -e "$NEW_SUBSTRATE"; then
        substrate_state="not moved"
      else
        substrate_state="ambiguous"
      fi
      echo "ESCALATE MANUALLY: stop all failed twice; rollback did NOT move either checkout back; half-moved state is: packages=$packages_state, substrate=$substrate_state" >&2
      test "$recovery_failed" -eq 0 || \
        echo "ESCALATE MANUALLY: start all or RUNNING doctor verification also failed" >&2
      return 1
    fi
  fi
  for moved_root in "$NEW_PACKAGES" "$NEW_SUBSTRATE"; do
    if test -d "$moved_root"; then
      held=$(mktemp)
      lsof +D "$moved_root" >"$held" 2>/dev/null || true
      if test -s "$held"; then
        cat "$held"; rm -f "$held"
        echo "ROLLBACK ABORTED: $moved_root is still held" >&2
        return 1
      fi
      rm -f "$held"
    fi
  done

  if test -d "$NEW_SUBSTRATE"; then
    test ! -e "$OLD_SUBSTRATE"
    mv "$NEW_SUBSTRATE" "$OLD_SUBSTRATE"
  fi
  if test -d "$NEW_PACKAGES"; then
    test ! -e "$OLD_PACKAGES"
    mv "$NEW_PACKAGES" "$OLD_PACKAGES"
  fi
  cp "$config_backup_p3" "$OLD_PACKAGES/.claude/skills/dogfood-github-devloop/dogfood.config.sh"

  for repo in packages substrate; do
    upper=$(printf '%s' "$repo" | tr '[:lower:]' '[:upper:]')
    eval "old_root=\$OLD_$upper"
    inventory="$HOME/.fkst/backups/p3-$repo-worktrees-before.txt"
    git -C "$old_root" worktree repair
    while IFS= read -r old_wt; do
      test -e "$old_wt"
      git -C "$old_root" worktree repair "$old_wt"
      git -C "$old_root" -c core.quotePath=false worktree list --porcelain |
        awk '/^worktree / {print substr($0,10)}' | grep -Fx -- "$old_wt"
    done <"$inventory"
  done

  DOGFOOD="$OLD_PACKAGES/.claude/skills/dogfood-github-devloop/dogfood.sh"
  "$DOGFOOD" start all
  assert_doctor_green
  assert_board_green
  rollback_counts=$(mktemp)
  snapshot_durable_counts "$rollback_counts" "$OLD_BIN" \
    "packages:$OLD_DUR_PACKAGES" \
    "substrate:$OLD_DUR_SUBSTRATE" \
    "website:$OLD_DUR_WEBSITE"
  cmp "$p3_counts_before" "$rollback_counts"
  rm -f "$rollback_counts"
}
```

Move both checkouts and perform every remaining operation inside one guarded
block. A partial two-checkout move and failures in config editing, repair, path
assertion, start, doctor, board, durable verification, or final location check
all invoke `p3_rollback`.

```sh
set -eu
set +e
(
  set -eu
  mv "$OLD_PACKAGES" "$NEW_PACKAGES"
  mv "$OLD_SUBSTRATE" "$NEW_SUBSTRATE"

  DOGFOOD="$NEW_PACKAGES/.claude/skills/dogfood-github-devloop/dogfood.sh"
  CONFIG="$NEW_PACKAGES/.claude/skills/dogfood-github-devloop/dogfood.config.sh"
  ${EDITOR:?set EDITOR} "$CONFIG"

  relocate_source_path() {
    case "$1/" in
      "$OLD_SUBSTRATE"/*) printf '%s\n' "$NEW_SUBSTRATE${1#"$OLD_SUBSTRATE"}" ;;
      *) printf '%s\n' "$1" ;;
    esac
  }
  EXPECTED_SUBSTRATE_SRC=$(relocate_source_path "$OLD_SUBSTRATE_SRC")
  EXPECTED_BIN=$(relocate_source_path "$OLD_BIN")

  for repo in packages substrate; do
    upper=$(printf '%s' "$repo" | tr '[:lower:]' '[:upper:]')
    eval "old_root=\$OLD_$upper"
    eval "new_root=\$NEW_$upper"
    inventory="$HOME/.fkst/backups/p3-$repo-worktrees-before.txt"
    git -C "$new_root" worktree repair
    while IFS= read -r old_wt; do
      case "$old_wt/" in
        "$old_root"/*) wt="$new_root${old_wt#"$old_root"}" ;;
        *) wt=$old_wt ;;
      esac
      test -e "$wt"
      git -C "$new_root" worktree repair "$wt"
      git -C "$new_root" -c core.quotePath=false worktree list --porcelain |
        awk '/^worktree / {print substr($0,10)}' | grep -Fx -- "$wt"
    done <"$inventory"
  done

  config_out=$(mktemp)
  "$DOGFOOD" config >"$config_out"
  cat "$config_out"
  test "$(resolved_from "$config_out" DOGFOOD_ROOT)" = "$OLD_DOGFOOD_ROOT"
  test "$(resolved_dur_from "$config_out" packages)" = "$OLD_DUR_PACKAGES"
  test "$(resolved_dur_from "$config_out" substrate)" = "$OLD_DUR_SUBSTRATE"
  test "$(resolved_dur_from "$config_out" website)" = "$OLD_DUR_WEBSITE"
  test "$(resolved_from "$config_out" RATE_POOL)" = "$OLD_RATE_POOL"
  test "$(resolved_from "$config_out" LOGDIR)" = "$OLD_LOGDIR"
  test "$(resolved_from "$config_out" SUBSTRATE_SRC)" = "$EXPECTED_SUBSTRATE_SRC"
  test "$(resolved_from "$config_out" BIN)" = "$EXPECTED_BIN"
  for value in \
    "$(resolved_from "$config_out" DOGFOOD_ROOT)" \
    "$(resolved_dur_from "$config_out" packages)" \
    "$(resolved_dur_from "$config_out" substrate)" \
    "$(resolved_dur_from "$config_out" website)" \
    "$(resolved_from "$config_out" RATE_POOL)" \
    "$(resolved_from "$config_out" LOGDIR)" \
    "$(resolved_from "$config_out" SUBSTRATE_SRC)" \
    "$(resolved_from "$config_out" BIN)"
  do
    assert_under "$value" "$HOME/.fkst"
  done
  rm -f "$config_out"

  "$DOGFOOD" start all
  assert_doctor_green
  assert_board_green
  p3_counts_after=$(mktemp)
  snapshot_durable_counts "$p3_counts_after" "$EXPECTED_BIN" \
    "packages:$OLD_DUR_PACKAGES" \
    "substrate:$OLD_DUR_SUBSTRATE" \
    "website:$OLD_DUR_WEBSITE"
  cmp "$p3_counts_before" "$p3_counts_after"
  rm -f "$p3_counts_after"

  cd "$NEW_PACKAGES"
  test "$(pwd -P)" = "$NEW_PACKAGES"
)
move_status=$?
set -e
if test "$move_status" -ne 0; then
  p3_rollback
  exit 1
fi
```

The final doctor and board checks are read-only positive gates: every target
must be current and running, every repository section must be present, and every
section must contain a running supervise line. Start a fresh operator session
only after the final `pwd -P` prints the absolute
`~/.fkst/src/fkst-packages` path. As in P2, the durable-count comparison is a
post-restart sanity check against a quiescent baseline, not byte-level proof.

## P1b: Retire Legacy Worktree Directories (After Concurrent Sessions End)

Audit each legacy worktree separately. A bundle cannot carry working-tree or
stash state, so use the same archive-or-explicit-loss rule as P1 before removing
the worktree. Expected result: clean tracked/untracked state, an empty shared
stash, ignored files archived or explicitly accepted-lost, no process holding
the worktree, and a verified bundle of the worktree tip. Any failure aborts
`git worktree remove`.

```sh
set -eu
OWNER=/absolute/path/to/main/checkout
WORKTREE=/absolute/path/to/legacy/worktree
BACKUP_DIR="$HOME/.fkst/backups"
IGNORED_POLICY=archive                 # archive | accept-loss
ACCEPT_IGNORED_LOSS=no                # set yes only with accept-loss

git -C "$OWNER" fetch --all --prune
git -C "$OWNER" -c core.quotePath=false worktree list --porcelain |
  awk '/^worktree / {print substr($0,10)}' | grep -Fx -- "$WORKTREE"
test -z "$(git -C "$WORKTREE" status --porcelain)"
test -z "$(git -C "$WORKTREE" stash list)"

mkdir -p "$BACKUP_DIR"
ignored_list=$(mktemp)
git -C "$WORKTREE" ls-files --others --ignored --exclude-standard -z >"$ignored_list"
if test -s "$ignored_list"; then
  case "$IGNORED_POLICY" in
    archive)
      ignored_tar="$BACKUP_DIR/$(basename "$WORKTREE")-ignored-$(date +%Y%m%dT%H%M%S).tar"
      tar -C "$WORKTREE" --null -T "$ignored_list" -cf "$ignored_tar"
      tar -tf "$ignored_tar" >/dev/null
      ;;
    accept-loss)
      test "$ACCEPT_IGNORED_LOSS" = yes
      ;;
    *) exit 1 ;;
  esac
fi
rm -f "$ignored_list"

held=$(mktemp)
lsof +D "$WORKTREE" >"$held" 2>/dev/null || true
test ! -s "$held"
rm -f "$held"

bundle="$BACKUP_DIR/$(basename "$WORKTREE")-tip-$(date +%Y%m%dT%H%M%S).bundle"
git -C "$WORKTREE" bundle create "$bundle" HEAD
git bundle verify "$bundle"
git -C "$OWNER" worktree remove "$WORKTREE"
test ! -e "$WORKTREE"
```

## Operational Facts

Linked Git worktrees store absolute paths in both directions: the linked
worktree's `.git` file points to the parent repository metadata, and the parent
repository's `.git/worktrees/<name>/gitdir` points back to the linked worktree.
Moving either side breaks those links. `git worktree repair` is Git's supported
mechanism for repairing them after a move.

redb durable roots are plain directories and are safe to move while all users
are stopped, followed by restart from the moved path. They are not disposable
runtime scratch. Recreating a durable root at a fresh path abandons its pending
and dead-letter records and can strand in-flight events; always stop, move the
existing directory, update configuration, explicitly start stopped targets,
and compare counts from the authoritative `observe --json` output.

⟦AI:FKST⟧
