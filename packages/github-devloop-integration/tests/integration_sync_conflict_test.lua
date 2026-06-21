local h = require("tests.devloop_helpers")
local cache_seed_helpers = require("tests.cache_seed_helpers")
local t = h.t
local core = h.core
local _ = cache_seed_helpers

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

local function seed_cache(key, value, run_opts)
  return t.run_department("departments/test_cache_seed/main.lua", {
    queue = "cache_seed",
    payload = {
      key = key,
      value = value,
    },
  }, run_opts)
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
  t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
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

local function codex_calls()
  local calls = {}
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find("codex exec", 1, true) ~= nil then
      table.insert(calls, call)
    end
  end
  return calls
end

local function assert_sync_conflict_worktree_call()
  local calls = codex_calls()
  t.eq(#calls, 1)
  t.is_true(calls[1].rendered:find(" -C ", 1, true) ~= nil)
  t.is_true(calls[1].rendered:find("/worktrees/sync-owner-repo-dev-integration-dev-", 1, true) ~= nil)
  t.is_nil(calls[1].rendered:find("/judgment-worktrees/", 1, true))
  t.is_true(calls[1].stdin:find("isolated runtime branch-sync worktree", 1, true) ~= nil)
  t.is_true(calls[1].stdin:find("not the supervise source checkout", 1, true) ~= nil)
  t.is_true(calls[1].stdin:find("Do not clone, checkout another branch", 1, true) ~= nil)
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
    assert_sync_conflict_worktree_call()
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

  test_sync_conflict_leftover_conflict_at_attempt_cap_escalates_without_failure = function()
    local payload = event()
    local remaining = "100644 abc 1\tcore.lua\n"
    local fingerprint = core.sync_conflict_fingerprint(payload, remaining)
    local run_opts = opts("sync-conflict-leftover-terminal", "1")
    seed_cache(core.sync_conflict_attempt_key(payload, fingerprint), tostring(core.max_sync_conflict_attempts() - 1), run_opts)
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    t.mock_command("codex exec", { stdout = "done", stderr = "", exit_code = 0 })
    t.mock_command("ls-files -u", { stdout = remaining, stderr = "", exit_code = 0 })
    mock_cleanup()

    local result = run_conflict(payload, run_opts)
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
    t.eq(h.count_calls("commit -F"), 0)
    local create = h.find_raise(result.raises, "github-proxy.github_issue_create_request")
    t.is_true(create ~= nil)
    t.is_true(create.payload.body:find("Attempt: " .. tostring(core.max_sync_conflict_attempts()), 1, true) ~= nil)
    t.is_true(create.payload.body:find("Reason: sync conflict remains unresolved after codex completed", 1, true) ~= nil)
    t.is_true(create.payload.dedup_key:find("sync-conflict-escalation", 1, true) ~= nil)
  end,

  test_sync_conflict_attempt_cap_escalates_before_codex = function()
    local payload = event()
    local remaining = "100644 abc 1\tcore.lua\n"
    local fingerprint = core.sync_conflict_fingerprint(payload, remaining)
    local run_opts = opts("sync-conflict-pre-codex-terminal", "1")
    seed_cache(core.sync_conflict_attempt_key(payload, fingerprint), tostring(core.max_sync_conflict_attempts()), run_opts)
    mock_fetch_and_heads()
    mock_conflicting_worktree()
    mock_cleanup()

    local result = run_conflict(payload, run_opts)
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("codex exec"), 0)
    t.eq(h.count_calls("commit -F"), 0)
    t.eq(h.count_calls("push origin HEAD:refs/heads/"), 0)
    local create = h.find_raise(result.raises, "github-proxy.github_issue_create_request")
    t.is_true(create ~= nil)
    t.is_true(create.payload.body:find("Attempt: " .. tostring(core.max_sync_conflict_attempts()), 1, true) ~= nil)
    t.is_true(create.payload.body:find("Reason: sync conflict retry budget already exhausted before codex", 1, true) ~= nil)
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
