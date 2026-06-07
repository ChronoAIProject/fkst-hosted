local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core

local function event(extra)
  local payload = {
    schema = "github-devloop.v1",
    repo = "owner/repo",
    upstream_branch = "dev",
    integration_branch = "integration/dev",
    upstream_sha = "aaaa1111",
    integration_sha = "bbbb2222",
    dedup_key = core.branch_sync_dedup_key("owner/repo", "dev", "integration/dev", "aaaa1111"),
    source_ref = core.branch_sync_source_ref("owner/repo", "dev", "integration/dev"),
  }
  for key, value in pairs(extra or {}) do
    payload[key] = value
  end
  return payload
end

local function opts(name, write)
  return {
    env = {
      FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/github-devloop/" .. tostring(now()) .. "/" .. tostring(name),
      FKST_GITHUB_WRITE = write or "1",
      FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
    },
  }
end

local function run_conflict(payload, run_opts)
  return t.run_department("departments/sync_conflict/main.lua", {
    queue = "devloop_sync_conflict",
    payload = payload or event(),
  }, run_opts or opts("sync-conflict"))
end

local function mock_fetch_and_heads(upstream_sha, integration_sha)
  t.mock_command("git fetch 'origin' 'dev'", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("refs/remotes/'origin'/'dev'^{commit}", { stdout = (upstream_sha or "aaaa1111") .. "\n", stderr = "", exit_code = 0 })
  t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", { stdout = (integration_sha or "bbbb2222") .. "\n", stderr = "", exit_code = 0 })
end

local function mock_conflicting_worktree()
  t.mock_command("merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 1 })
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-rt", stderr = "", exit_code = 0 })
  t.mock_command("git worktree add --detach", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("merge --no-ff --no-commit", { stdout = "", stderr = "conflict", exit_code = 1 })
  t.mock_command("ls-files -u", { stdout = "100644 abc 1\tcore.lua\n", stderr = "", exit_code = 0 })
end

local function mock_successful_codex_resolution()
  t.mock_command("codex exec", { stdout = "resolved", stderr = "", exit_code = 0 })
  t.mock_command("ls-files -u", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("diff --check", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("diff --cached --check", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("git -C", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("ls-files -u", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("diff --cached --check", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("commit -F", { stdout = "[detached cccc3333] Sync dev into integration/dev\n", stderr = "", exit_code = 0 })
end

local function mock_real_push(integration_recheck, pushed_head)
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
  t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", { stdout = (integration_recheck or "bbbb2222") .. "\n", stderr = "", exit_code = 0 })
  if integration_recheck == nil or integration_recheck == "bbbb2222" then
    t.mock_command("rev-parse HEAD", { stdout = (pushed_head or "cccc3333") .. "\n", stderr = "", exit_code = 0 })
    t.mock_command("push origin HEAD:refs/heads/", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("git fetch 'origin' 'integration/dev'", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("refs/remotes/'origin'/'integration/dev'^{commit}", { stdout = (pushed_head or "cccc3333") .. "\n", stderr = "", exit_code = 0 })
  end
end

local function mock_cleanup()
  t.mock_command("git worktree remove --force", { stdout = "", stderr = "", exit_code = 0 })
end

return {
  test_sync_conflict_codex_success_commits_and_guarded_pushes = function()
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    mock_successful_codex_resolution()
    mock_real_push()
    mock_cleanup()

    local result = run_conflict(event(), opts("sync-conflict-success", "1"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(h.count_calls("codex exec"), 1)
    t.eq(h.count_calls("ls-files -u"), 3)
    t.eq(h.count_calls("commit -F"), 1)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 1)
  end,

  test_sync_conflict_codex_failure_errors_without_push = function()
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    t.mock_command("codex exec", { stdout = "", stderr = "failed", exit_code = 1 })
    mock_cleanup()

    local result = run_conflict(event(), opts("sync-conflict-codex-failure", "1"))
    t.eq(result.exit_code, 1)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
  end,

  test_sync_conflict_leftover_conflict_errors_without_push = function()
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    t.mock_command("codex exec", { stdout = "done", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = "100644 abc 1\tcore.lua\n", stderr = "", exit_code = 0 })
    mock_cleanup()

    local result = run_conflict(event(), opts("sync-conflict-leftover", "1"))
    t.eq(result.exit_code, 1)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
  end,

  test_sync_conflict_staged_conflict_marker_errors_without_commit_or_push = function()
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    t.mock_command("codex exec", { stdout = "done", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --check", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --cached --check", {
      stdout = "core.lua:1: leftover conflict marker\n",
      stderr = "",
      exit_code = 2,
    })
    mock_cleanup()

    local result = run_conflict(event(), opts("sync-conflict-staged-marker", "1"))
    t.eq(result.exit_code, 1)
    t.eq(h.count_calls("commit -F"), 0)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
  end,

  test_sync_conflict_staged_whitespace_after_add_errors_without_commit_or_push = function()
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    t.mock_command("codex exec", { stdout = "done", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --check", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --cached --check", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("git -C", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --cached --check", {
      stdout = "core.lua:2: trailing whitespace.\n",
      stderr = "",
      exit_code = 2,
    })
    mock_cleanup()

    local result = run_conflict(event(), opts("sync-conflict-staged-after-add", "1"))
    t.eq(result.exit_code, 1)
    t.eq(h.count_calls("commit -F"), 0)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
  end,

  test_sync_conflict_unmerged_reappears_after_add_errors_without_commit_or_push = function()
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    t.mock_command("codex exec", { stdout = "done", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --check", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("diff --cached --check", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("git -C", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = "100644 abc 1\tcore.lua\n", stderr = "", exit_code = 0 })
    mock_cleanup()

    local result = run_conflict(event(), opts("sync-conflict-unmerged-after-add", "1"))
    t.eq(result.exit_code, 1)
    t.eq(h.count_calls("commit -F"), 0)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
  end,

  test_sync_conflict_integration_head_moved_before_push_skips_unsafe_push = function()
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    mock_successful_codex_resolution()
    mock_real_push("dddd4444")
    mock_cleanup()

    local result = run_conflict(event(), opts("sync-conflict-head-moved", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("commit -F"), 1)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
  end,
}
