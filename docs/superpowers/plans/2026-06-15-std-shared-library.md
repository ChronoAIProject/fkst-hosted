# `std` Shared Library Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up a repo-level shared Lua library `std/`, requirable from every package via a per-package symlink, with conformance guarding that only `std` (not peer packages) is shared.

**Architecture:** `std/` lives once at repo root. Each package gets a git-committed symlink `packages/<pkg>/std -> ../../std`, so the engine's owner-scoped `package.path` (= package root) resolves `require("std.<m>")` into the shared tree. Tier S (substrate-contract) and Tier R (repo-domain) modules cohabit `std/`; Tier S is shaped for later promotion into the substrate. Verified by spike: a symlinked `std` module resolves via `require` and passes flat single-root conformance.

**Tech Stack:** Lua (package behavior layer), the `fkst-framework` engine test/conformance runner via `scripts/run.sh`, Python `scripts/check_repo.py` (G-gates).

Spec: `docs/superpowers/specs/2026-06-14-std-shared-library-design.md`

---

### Task 1: Create `std/` and prove it resolves in every package

**Files:**
- Create: `std/init.lua`
- Create: `std/strings.lua` (first Tier R inhabitant — generic utils)
- Create symlink (per package): `packages/<pkg>/std -> ../../std` for `github-devloop`, `github-proxy`, `consensus`, `autochrono`, `github-autochrono`
- Test: `std/tests/strings_test.lua`
- Test (resolution probe, per package): `packages/<pkg>/tests/std_resolves_test.lua`

- [ ] **Step 1: Write the failing resolution probe** for one package

`packages/github-proxy/tests/std_resolves_test.lua`:
```lua
local std = require("std")
local t = fkst.test
return {
  test_std_root_resolves = function()
    t.eq(type(std), "table")
    t.eq(std.version, "0")
  end,
}
```

- [ ] **Step 2: Run it, verify it fails** (no `std` yet)

Run: `scripts/run.sh test github-proxy`
Expected: FAIL — `module 'std' not found`

- [ ] **Step 3: Create `std/init.lua`**

```lua
-- std: repo-level shared library for fkst-packages.
-- Tier S (substrate-contract) and Tier R (repo-domain) modules live here and
-- are required as `std.<module>`. Vendored into each package via a committed
-- `packages/<pkg>/std -> ../../std` symlink (owner-scoped package.path resolves it).
return {
  version = "0",
}
```

- [ ] **Step 4: Create the per-package symlink**

```bash
for pkg in github-devloop github-proxy consensus autochrono github-autochrono; do
  ln -s ../../std "packages/$pkg/std"
done
git add packages/*/std
```

- [ ] **Step 5: Run the probe, verify it passes**

Run: `scripts/run.sh test github-proxy`
Expected: PASS (`test_std_root_resolves`)

- [ ] **Step 6: Add the resolution probe to every package** (copy the test into each `packages/<pkg>/tests/std_resolves_test.lua`) and run the full suite

Run: `scripts/run.sh test`
Expected: every package PASS; this is the fail-closed guarantee that a missing/broken symlink is caught (§9 of the spec).

- [ ] **Step 7: Seed the first Tier R module with a real test**

`std/strings.lua`:
```lua
local S = {}
function S.trim(value)
  return (tostring(value or ""):gsub("^%s+", ""):gsub("%s+$", ""))
end
return S
```
`std/tests/strings_test.lua`:
```lua
local strings = require("std.strings")
local t = fkst.test
return {
  test_trim_strips_both_ends = function()
    t.eq(strings.trim("  hi  "), "hi")
    t.eq(strings.trim(nil), "")
  end,
}
```

> Note: `std/tests/*_test.lua` is discovered when `std/` is reached through a
> package symlink. Confirm the engine test runner enumerates it under at least
> one package root; if it does not (runner only scans `<root>/tests`), move
> `std`'s tests to be exercised via a dedicated `packages/std-selftest/`
> harness package, or assert std modules through each package's probe. Decide
> during execution based on the runner's actual discovery.

- [ ] **Step 8: Run and commit**

Run: `scripts/run.sh test`
Expected: all green.
```bash
git add std packages/*/std packages/*/tests/std_resolves_test.lua
git commit -m "feat(std): repo-level shared lib + per-package symlink + resolution probe"
```

---

### Task 2: Doctrine edit + peer-require guard

**Files:**
- Modify: `CLAUDE.md` (the "包结构约定 / 包内共享库" paragraph)
- Modify: `scripts/check_repo.py` (add `check_cross_package_require`)
- Test: `scripts/check_repo_test.py`

- [ ] **Step 1: Write the failing guard test**

In `scripts/check_repo_test.py`, add a case: a package file containing
`require("github-proxy.core")` (peer require) must produce a `G9` violation;
`require("std.saga")` must NOT.

- [ ] **Step 2: Run it, verify it fails**

Run: `python3 -B scripts/check_repo_test.py`
Expected: FAIL (no such check yet).

- [ ] **Step 3: Implement `check_cross_package_require`** in `scripts/check_repo.py`

Scan every `packages/<pkg>/**/*.lua` for `require("<name>...")`. Allow
`require("core...")`, `require("departments...")`, `require("std...")`, and
relative within-package names. Flag any `require("<sibling-pkg>...")` where
`<sibling-pkg>` is another package dir name → `G9` violation
("peer cross-package require forbidden; share via std/"). Register it in `main()`
alongside the other checks.

- [ ] **Step 4: Run the test, verify it passes**

Run: `python3 -B scripts/check_repo_test.py`
Expected: PASS.

- [ ] **Step 5: Edit CLAUDE.md doctrine**

Replace the prohibition in the "包内共享库放 package-root" paragraph with:
> 包内共享库放 package-root `core.lua`;跨包共享放 repo-root `std/`(单向、分层,
> 经 `packages/<pkg>/std -> ../../std` 符号链接引入,`require("std.<m>")`)。
> **禁 peer 跨包 require(A→B 内部);允许唯一 blessed 共享库根(all→std)。**
> `std` 不是 manifest / 版本解析。

- [ ] **Step 6: Run full check + commit**

Run: `python3 -B scripts/check_repo.py` and `scripts/run.sh test`
Expected: green.
```bash
git add CLAUDE.md scripts/check_repo.py scripts/check_repo_test.py
git commit -m "feat(std): allow blessed std require, forbid peer cross-package require (G9) + doctrine"
```

---

### Task 3: Drain the first real duplicated helper into `std`

Target the helper duplicated in the most packages: `persistence_class` (×3) is
special (it must stay per-package as the saga identity) — instead drain a pure
util. Use `shell_single_quote` (×2: github-devloop, github-proxy).

**Files:**
- Create: `std/shell.lua`
- Test: `std/tests/shell_test.lua` (or via a package probe per Task 1 Step 7 decision)
- Modify: the two packages' `core/*.lua` that define `shell_single_quote` to `require("std.shell")`

- [ ] **Step 1: Write the failing test** for `std.shell.single_quote` with the exact
  semantics of the existing implementations (read both first; they must match).

```lua
local shell = require("std.shell")
local t = fkst.test
return {
  test_single_quote_wraps_and_escapes = function()
    t.eq(shell.single_quote("a'b"), [['a'\''b']])
    t.eq(shell.single_quote("plain"), [['plain']])
  end,
}
```

- [ ] **Step 2: Run, verify it fails**

Run: `scripts/run.sh test`
Expected: FAIL.

- [ ] **Step 3: Implement `std/shell.lua`** by lifting the existing
  `shell_single_quote` body verbatim (confirm both packages' bodies are
  byte-identical first; if they differ, reconcile to the safer one and note why).

- [ ] **Step 4: Replace both packages' copies** with `require("std.shell").single_quote`
  (keep the public `M.shell_single_quote` name in each package's `core` by
  assigning it from `std`, so call sites are untouched).

- [ ] **Step 5: Run full suite, verify green**

Run: `scripts/run.sh test`
Expected: all packages PASS (behavior unchanged).

- [ ] **Step 6: Add ratchet note + commit**

```bash
git add std packages/github-devloop packages/github-proxy
git commit -m "refactor(std): drain shell_single_quote into std/shell (DRY, no behavior change)"
```

---

### Task 4: Confirm CI (ubuntu) resolves committed symlinks

**Files:** none (verification task) — possibly `.github/workflows/ci.yml` if a checkout option is needed.

- [ ] **Step 1:** Push the branch and confirm the CI `test` job is green (symlinks
  resolve on ubuntu, conformance passes). If git checked out symlinks as plain
  files, set `core.symlinks=true` handling or add a CI step to recreate them; the
  resolution probe (Task 1) will fail loudly if so — that is the intended
  fail-closed signal.

- [ ] **Step 2:** Record the CI result (command + run URL) in the PR body.

---

## Self-Review

- **Spec coverage:** §3 tiers (Task 1 seeds both), §4 doctrine (Task 2), §5 symlink
  vendoring (Task 1), §7 conformance accounting (Task 1 probe + Task 2 guard), §8
  drain (Task 3), §9 testing (Task 1 probe), §10 R1 CI (Task 4). Tier S promotion
  to substrate is out of scope (future, per spec §6).
- **Placeholders:** none; the one open decision (std test discovery) is explicit in
  Task 1 Step 7 with a concrete fallback.
- **Naming:** `require("std.<m>")`, `packages/<pkg>/std`, `G9`, `std/init.lua`
  used consistently.

⟦AI:FKST⟧
