local devloop_base = require("devloop.base")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local gh_argv = require("testkit.gh_argv_mock")
local m_builders = require("devloop.markers.builders")
local opts = h.opts
local fixing = h.fixing
local run_fix = h.run_fix
local mock_issue_fix_for_event = h.mock_issue_fix_for_event
local mock_pr_fix = h.mock_pr_fix
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local mock_git_push = h.mock_git_push
local mock_existing_fix_worktree = h.mock_existing_fix_worktree
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local count_calls = h.count_calls
local find_raise = h.find_raise
local find_causal_raise = h.find_causal_raise

return {
  test_mergeable_conflicting_fix_skips_pr_merge_ref_verification = function()
    local event = fixing({ gate_baseline_sha = "abc123", gate_failure_excerpt = "mergeable-conflicting" })
    local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
    local reject_comment = "github-devloop merge gate failed: mergeable-conflicting"
      .. "\n" .. core.state_marker(event.proposal_id, "fixing", event.version)
      .. "\n" .. m_builders.merge_gate_marker(event.proposal_id,
        event.pr_number,
        event.version,
        event.review_proposal_id,
        event.review_dedup_key,
        event.reviewed_head_sha,
        event.gate_baseline_sha,
        "mergeable-conflicting"
      )
    local origin_marker = m_builders.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    local worktree = mock_existing_fix_worktree(branch, "def456", nil, {
      sha = "abc123",
      exit_code = 1,
      stdout = "",
      stderr = "CONFLICT (content): Merge conflict in packages/github-devloop/core.lua\n",
      unmerged_stdout = "100644 abc123 1\tpackages/github-devloop/core.lua\n",
      post_codex_unmerged_stdout = "",
    })
    mock_implement_codex(0, "resolved integration conflict")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_git_push(branch)
    mock_pr_fix({ origin_marker }, branch, "feedface")

    local result = run_fix(event, opts("fix-conflicting-skips-pr-merge-ref", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(find_causal_raise(result, "devloop_reviewing").payload.version, core.next_fix_version(event.version))
    t.eq(count_calls("git fetch 'origin' 'refs/pull/7/merge'"), 0)
    t.eq(count_calls("git rev-parse --verify FETCH_HEAD^{commit}"), 0)
    t.eq(count_calls("git fetch 'origin' 'dev'"), 0)
    t.eq(count_calls("refs/remotes/'origin'/'dev'^{commit}"), 0)
    t.eq(count_calls("merge --no-edit 'abc123'"), 1)

    local saw_conflict_prompt = false
    for _, call in ipairs(t.command_calls()) do
      if call.rendered:find("codex exec", 1, true) ~= nil
        and tostring(call.stdin or ""):find("sync_conflict target_branch=dev target_sha=abc123", 1, true) ~= nil
        and tostring(call.stdin or ""):find("packages/github-devloop/core.lua", 1, true) ~= nil then
        saw_conflict_prompt = true
      end
    end
    t.eq(saw_conflict_prompt, true)
    t.is_true(worktree ~= nil)
  end,

  test_fix_merges_gate_baseline_before_codex = function()
    local event = fixing({ gate_baseline_sha = "abc123", gate_failure_excerpt = "own-ci-red" })
    local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
    local reject_comment = "github-devloop merge gate failed: own-ci-red"
      .. "\n" .. core.state_marker(event.proposal_id, "fixing", event.version)
      .. "\n" .. m_builders.merge_gate_marker(event.proposal_id,
        event.pr_number,
        event.version,
        event.review_proposal_id,
        event.review_dedup_key,
        event.reviewed_head_sha,
        event.gate_baseline_sha,
        "own-ci-red"
      )
    local origin_marker = m_builders.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    local worktree = mock_existing_fix_worktree(branch, "def456", nil, {
      sha = "abc123",
      exit_code = 1,
      stdout = "",
      stderr = "CONFLICT (content): Merge conflict in packages/github-devloop/core.lua\n",
      unmerged_stdout = "100644 abc123 1\tpackages/github-devloop/core.lua\n",
      post_codex_unmerged_stdout = "",
    })
    mock_implement_codex(0, "resolved owned CI failure")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_git_push(branch)
    mock_pr_fix({ origin_marker }, branch, "feedface")

    local result = run_fix(event, opts("fix-gate-baseline-before-codex", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(find_causal_raise(result, "devloop_reviewing").payload.version, core.next_fix_version(event.version))

    local merge_index = nil
    local codex_index = nil
    for index, call in ipairs(t.command_calls()) do
      if gh_argv.argv_contains(call, { "git", "-C", worktree, "merge", "--no-edit", "abc123" }) then
        merge_index = index
      elseif call.rendered:find("codex exec", 1, true) ~= nil then
        codex_index = index
      end
    end
    t.is_true(merge_index ~= nil)
    t.is_true(codex_index ~= nil)
    t.is_true(merge_index < codex_index)
    local saw_conflict_prompt = false
    for _, call in ipairs(t.command_calls()) do
      if call.rendered:find("codex exec", 1, true) ~= nil
        and tostring(call.stdin or ""):find("sync_conflict target_branch=dev target_sha=abc123", 1, true) ~= nil
        and tostring(call.stdin or ""):find("packages/github-devloop/core.lua", 1, true) ~= nil then
        saw_conflict_prompt = true
      end
    end
    t.eq(saw_conflict_prompt, true)
    t.eq(count_calls("git fetch 'origin' 'dev'"), 0)
    t.eq(count_calls("git fetch 'origin' 'refs/pull/7/merge'"), 0)
    t.eq(count_calls("refs/remotes/'origin'/'dev'^{commit}"), 0)
    t.eq(count_calls("merge --no-edit 'abc123'"), 1)
    t.eq(count_calls("ls-files -u"), 2)
  end,

  test_fix_ignores_unchanged_pytest_separator_outside_candidate_diff = function()
    local event = fixing({ gate_baseline_sha = "abc123", gate_failure_excerpt = "own-ci-red" })
    local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
    local reject_comment = "github-devloop merge gate failed: own-ci-red"
      .. "\n" .. core.state_marker(event.proposal_id, "fixing", event.version)
      .. "\n" .. m_builders.merge_gate_marker(event.proposal_id,
        event.pr_number,
        event.version,
        event.review_proposal_id,
        event.review_dedup_key,
        event.reviewed_head_sha,
        event.gate_baseline_sha,
        "own-ci-red"
      )
    local origin_marker = m_builders.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("grep -n -I -E", {
      stdout = "IMPLEMENT_REPORT.md:1:" .. string.rep("=", 25) .. " 6 passed in 66.21s (0:01:06) " .. string.rep("=", 25) .. "\n",
      stderr = "",
      exit_code = 0,
    })
    mock_existing_fix_worktree(branch, "def456", nil, {
      sha = "abc123",
      exit_code = 1,
      stdout = "",
      stderr = "CONFLICT (content): Merge conflict in packages/github-devloop/core.lua\n",
      unmerged_stdout = "100644 abc123 1\tpackages/github-devloop/core.lua\n",
      post_codex_unmerged_stdout = "",
    })
    mock_implement_codex(0, "resolved owned CI failure")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_git_push(branch)
    mock_pr_fix({ origin_marker }, branch, "feedface")

    local result = run_fix(event, opts("fix-unchanged-pytest-separator", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("grep -n -I -E"), 0)
    t.eq(count_calls("diff --check 'def456..feedface'"), 1)
    t.eq(count_calls("commit -m"), 1)
    t.eq(count_calls("git push origin"), 1)
  end,

  test_fix_errors_on_conflict_marker_in_candidate_diff = function()
    local event = fixing({ gate_baseline_sha = "abc123", gate_failure_excerpt = "own-ci-red" })
    local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
    local reject_comment = "github-devloop merge gate failed: own-ci-red"
      .. "\n" .. core.state_marker(event.proposal_id, "fixing", event.version)
      .. "\n" .. m_builders.merge_gate_marker(event.proposal_id,
        event.pr_number,
        event.version,
        event.review_proposal_id,
        event.review_dedup_key,
        event.reviewed_head_sha,
        event.gate_baseline_sha,
        "own-ci-red"
      )
    local origin_marker = m_builders.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    mock_existing_fix_worktree(branch, "def456", nil, {
      candidate_diff_stdout = "packages/github-devloop/core.lua:1: leftover conflict marker\n",
      candidate_diff_exit_code = 2,
    })
    mock_implement_codex(0, "left a conflict marker")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")

    local result = run_fix(event, opts("fix-candidate-conflict-marker", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 1)
    t.is_true(tostring(result.error or ""):find("unresolved-conflict-markers", 1, true) ~= nil)
    t.eq(count_calls("diff --check 'def456..feedface'"), 1)
    t.eq(count_calls("status --porcelain"), 1)
    t.eq(count_calls("commit -m"), 1)
    t.eq(count_calls("git push origin"), 0)
  end,

  test_fix_checks_reviewed_head_through_ahead_plus_dirty_candidate = function()
    local event = fixing({ gate_baseline_sha = "abc123", gate_failure_excerpt = "own-ci-red" })
    local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
    local reject_comment = "github-devloop merge gate failed: own-ci-red"
      .. "\n" .. core.state_marker(event.proposal_id, "fixing", event.version)
      .. "\n" .. m_builders.merge_gate_marker(event.proposal_id,
        event.pr_number,
        event.version,
        event.review_proposal_id,
        event.review_dedup_key,
        event.reviewed_head_sha,
        event.gate_baseline_sha,
        "own-ci-red"
      )
    local origin_marker = m_builders.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    mock_existing_fix_worktree(branch, "beadface")
    mock_implement_codex(0, "added another repair")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_git_push(branch)
    mock_pr_fix({ origin_marker }, branch, "feedface")

    local result = run_fix(event, opts("fix-ahead-plus-dirty-candidate", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("diff --check 'def456..feedface'"), 1)
    t.eq(count_calls("git push origin"), 1)
  end,

  test_fix_errors_when_unmerged_index_remains_after_worker = function()
    local event = fixing({ gate_baseline_sha = "abc123", gate_failure_excerpt = "own-ci-red" })
    local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
    local reject_comment = "github-devloop merge gate failed: own-ci-red"
      .. "\n" .. core.state_marker(event.proposal_id, "fixing", event.version)
      .. "\n" .. m_builders.merge_gate_marker(event.proposal_id,
        event.pr_number,
        event.version,
        event.review_proposal_id,
        event.review_dedup_key,
        event.reviewed_head_sha,
        event.gate_baseline_sha,
        "own-ci-red"
      )
    local origin_marker = m_builders.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    mock_existing_fix_worktree(branch, "def456", nil, {
      sha = "abc123",
      exit_code = 1,
      stderr = "CONFLICT (content): Merge conflict in packages/github-devloop/core.lua\n",
      unmerged_stdout = "100644 abc123 1\tpackages/github-devloop/core.lua\n",
      post_codex_unmerged_stdout = "100644 abc123 1\tpackages/github-devloop/core.lua\n",
    })
    mock_implement_codex(0, "did not resolve the index")
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")

    local result = run_fix(event, opts("fix-unmerged-index-remains", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 1)
    t.is_true(tostring(result.error or ""):find("unresolved-merge-conflicts", 1, true) ~= nil)
    t.eq(count_calls("codex exec"), 1)
    t.eq(count_calls("grep -n -I -E"), 0)
    t.eq(count_calls("diff --check"), 0)
    t.eq(count_calls("status --porcelain"), 0)
    t.eq(count_calls("git push origin"), 0)
  end,
}
