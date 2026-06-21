local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core

local function opts(name, extra_env)
  return {
    env = {
      FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/github-devloop/" .. tostring(now()) .. "/" .. tostring(name),
      FKST_GITHUB_REPO = "owner/repo",
      FKST_DEVLOOP_UPSTREAM_BRANCH = "dev",
      FKST_DEVLOOP_INTEGRATION_BRANCH = "integration/dev",
      FKST_DEVLOOP_ROLLUP_MERGE = "auto",
      FKST_GITHUB_WRITE = "",
      FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
    },
  }
end

local function mock_env(write_mode, integration)
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', { stdout = integration or "integration/dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = write_mode or "", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', { stdout = integration or "integration/dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_ROLLUP_MERGE"', { stdout = "auto", stderr = "", exit_code = 0 })
end

local function run_scan(run_opts)
  return t.run_department("departments/sync_scan/main.lua", {
    queue = "devloop_branch_tick",
    payload = { schema = "github-devloop.branch-tick.v1" },
  }, run_opts or opts("sync-scan"))
end

local function mock_fetch_and_heads(upstream_sha, integration_sha)
  t.mock_command("git fetch 'origin' 'dev'", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("refs/remotes/'origin'/'dev'^{commit}", { stdout = upstream_sha .. "\n", stderr = "", exit_code = 0 })
  t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", { stdout = integration_sha .. "\n", stderr = "", exit_code = 0 })
end

local function mock_worktree_merge(exit_code, unmerged_stdout)
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-rt", stderr = "", exit_code = 0 })
  t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("git worktree add --detach", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("merge --no-ff --no-commit", { stdout = "", stderr = exit_code == 0 and "" or "conflict", exit_code = exit_code })
  if exit_code ~= 0 then
    t.mock_command("ls-files -u", { stdout = unmerged_stdout or "100644 abc 1\tcore.lua\n", stderr = "", exit_code = 0 })
  end
  t.mock_command("git worktree remove --force", { stdout = "", stderr = "", exit_code = 0 })
end

local function mock_worktree_fast_forward()
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-rt", stderr = "", exit_code = 0 })
  t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("git worktree add --detach", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("merge --ff-only", { stdout = "Updating bbbb2222..aaaa1111\nFast-forward\n", stderr = "", exit_code = 0 })
  t.mock_command("git worktree remove --force", { stdout = "", stderr = "", exit_code = 0 })
end

local function mock_tree_compare(equal)
  t.mock_command("git diff --quiet aaaa1111 bbbb2222", {
    stdout = "",
    stderr = "",
    exit_code = equal and 0 or 1,
  })
end

local function count_calls(needle)
  return h.count_calls(needle)
end

local function has_call(needle)
  return h.has_call(needle)
end

return {
  test_sync_scan_integration_equal_upstream_noops = function()
    mock_env("", "dev")
    local result = run_scan(opts("sync-same", { FKST_DEVLOOP_INTEGRATION_BRANCH = "dev" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("git fetch"), 0)
  end,

  test_sync_scan_upstream_ancestor_noops = function()
    mock_env()
    mock_fetch_and_heads("aaaa1111", "bbbb2222")
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 0 })

    local result = run_scan()
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("git worktree add"), 0)
  end,

  test_sync_scan_clean_merge_real_mode_pushes_after_unchanged_head_recheck = function()
    mock_env("1")
    mock_fetch_and_heads("aaaa1111", "bbbb2222")
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    mock_tree_compare(false)
    mock_worktree_merge(0)
    t.mock_command("commit -F", { stdout = "[detached cccc3333] Sync dev into integration/dev\n", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
    t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", { stdout = "bbbb2222\n", stderr = "", exit_code = 0 })
    t.mock_command("rev-parse HEAD", { stdout = "cccc3333\n", stderr = "", exit_code = 0 })
    t.mock_command("push origin HEAD:refs/heads/", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", { stdout = "cccc3333\n", stderr = "", exit_code = 0 })

    local result = run_scan(opts("sync-clean-real", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("git -C"), 4)
    t.eq(count_calls("push origin HEAD:refs/heads/"), 1)
    t.eq(count_calls("refs/remotes/'origin'/'integration/dev'^{commit}"), 3)
    t.eq(has_call('result="clean"'), false)
  end,

  test_sync_scan_integration_ancestor_fast_forwards_without_merge_commit = function()
    mock_env("1")
    mock_fetch_and_heads("aaaa1111", "bbbb2222")
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 0 })
    mock_worktree_fast_forward()
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
    t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", { stdout = "bbbb2222\n", stderr = "", exit_code = 0 })
    t.mock_command("rev-parse HEAD", { stdout = "aaaa1111\n", stderr = "", exit_code = 0 })
    t.mock_command("push origin HEAD:refs/heads/", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", { stdout = "aaaa1111\n", stderr = "", exit_code = 0 })

    local result = run_scan(opts("sync-fast-forward-real", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("merge --ff-only"), 1)
    t.eq(count_calls("merge --no-ff --no-commit"), 0)
    t.eq(count_calls("commit -F"), 0)
    t.eq(count_calls("push origin HEAD:refs/heads/"), 1)
  end,

  test_sync_scan_tree_equal_real_mode_converges_with_force_with_lease = function()
    mock_env("1")
    mock_fetch_and_heads("aaaa1111", "bbbb2222")
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    mock_tree_compare(true)
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
    t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", { stdout = "bbbb2222\n", stderr = "", exit_code = 0 })
    mock_tree_compare(true)
    t.mock_command("git push origin aaaa1111:refs/heads/integration/dev --force-with-lease=refs/heads/integration/dev:bbbb2222", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", { stdout = "aaaa1111\n", stderr = "", exit_code = 0 })

    local result = run_scan(opts("sync-tree-equal-real", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("merge --no-ff --no-commit"), 0)
    t.eq(count_calls("--force-with-lease=refs/heads/integration/dev:bbbb2222"), 1)
    t.eq(count_calls("refs/remotes/'origin'/'integration/dev'^{commit}"), 3)
  end,

  test_sync_scan_tree_equal_dry_run_logs_without_push = function()
    mock_env("")
    mock_fetch_and_heads("aaaa1111", "bbbb2222")
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    mock_tree_compare(true)
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "", stderr = "", exit_code = 0 })

    local result = run_scan()
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("merge --no-ff --no-commit"), 0)
    t.eq(count_calls("--force-with-lease"), 0)
    t.eq(count_calls("git fetch 'origin' 'integration/dev'"), 1)
  end,

  test_sync_scan_tree_equal_real_mode_skips_when_integration_head_changed = function()
    mock_env("1")
    mock_fetch_and_heads("aaaa1111", "bbbb2222")
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    mock_tree_compare(true)
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
    t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", { stdout = "cccc3333\n", stderr = "", exit_code = 0 })

    local result = run_scan(opts("sync-tree-equal-head-changed", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--force-with-lease"), 0)
    t.eq(count_calls("git diff --quiet aaaa1111 bbbb2222"), 1)
  end,

  test_sync_scan_dry_run_clean_merge_never_pushes = function()
    mock_env("")
    mock_fetch_and_heads("aaaa1111", "bbbb2222")
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    mock_tree_compare(false)
    mock_worktree_merge(0)
    t.mock_command("commit -F", { stdout = "[detached cccc3333] Sync dev into integration/dev\n", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "", stderr = "", exit_code = 0 })

    local result = run_scan()
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("push origin HEAD:refs/heads/"), 0)
  end,

  test_sync_scan_conflict_raises_sync_conflict = function()
    mock_env("")
    mock_fetch_and_heads("aaaa1111", "bbbb2222")
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
    mock_tree_compare(false)
    mock_worktree_merge(1, "100644 abc 1\tcore.lua\n")

    local result = run_scan()
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local raised = h.find_raise(result.raises, "devloop_sync_conflict")
    t.eq(raised.payload.schema, "github-devloop.v1")
    t.eq(raised.payload.repo, "owner/repo")
    t.eq(raised.payload.upstream_branch, "dev")
    t.eq(raised.payload.integration_branch, "integration/dev")
    t.eq(raised.payload.upstream_sha, "aaaa1111")
    t.eq(raised.payload.integration_sha, "bbbb2222")
    t.eq(raised.payload.dedup_key, core.branch_sync_dedup_key("owner/repo", "dev", "integration/dev", "aaaa1111"))
    t.eq(raised.payload.source_ref.ref, "owner/repo#branch-sync/dev/integration/dev")
    t.eq(count_calls("push origin HEAD:refs/heads/"), 0)
  end,
}
