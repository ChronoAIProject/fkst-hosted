local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local merge_ready = h.merge_ready
local run_merge = h.run_merge
local mock_issue_merge = h.mock_issue_merge
local merge_comments = h.merge_comments
local mock_pr_merge_rollup = h.mock_pr_merge_rollup
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local fixing = h.fixing
local run_fix = h.run_fix
local mock_issue_fix_for_event = h.mock_issue_fix_for_event
local mock_pr_fix = h.mock_pr_fix
local mock_existing_fix_worktree = h.mock_existing_fix_worktree
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local mock_git_push = h.mock_git_push
local has_value = h.has_value
local count_calls = h.count_calls
local find_raise = h.find_raise

return {
  test_merge_ci_red_without_rollup_sha_degrades_to_current_integration_fix = function()
    local event = merge_ready()
    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge_rollup({ origin_marker }, '[{"__typename":"CheckRun","completedAt":"2026-06-03T02:04:04Z","conclusion":"FAILURE","detailsUrl":"https://example.invalid/checks/test","name":"test","startedAt":"2026-06-03T02:03:04Z","status":"COMPLETED","workflowName":"ci"}]', nil, nil, nil, nil, nil, nil, nil, nil, nil, "base999")

    local result = run_merge(event, opts("merge-ci-red", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    t.eq(count_calls("gh pr merge"), 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:fixing")
    local fixing_payload = find_raise(result.raises, "devloop_fixing").payload
    t.eq(fixing_payload.schema, "github-devloop.fixing.v1")
    t.eq(fixing_payload.gate_baseline_sha, nil)
    t.eq(fixing_payload.gate_failure_excerpt, "rollup-red: test: COMPLETED/FAILURE")
    local comment_body = find_raise(result.raises, "github-proxy.github_pr_comment_request").payload.body
    t.is_true(comment_body:find("fkst:github-devloop:merge-gate:v1", 1, true) ~= nil)
    t.is_true(comment_body:find("gate_baseline_sha", 1, true) == nil)
    t.is_true(comment_body:find("rollup-red: test: COMPLETED/FAILURE", 1, true) ~= nil)
    t.is_true(comment_body:find("Reproduce locally with `scripts/run.sh test`", 1, true) ~= nil)
    local fix_fact = core.merge_gate_fix_fact({ comment_body }, event.proposal_id, core.fix_version_from_review_version(event.version))
    t.is_true(fix_fact.review_reason:find("rollup-red: test: COMPLETED/FAILURE", 1, true) ~= nil)
    t.eq(fix_fact.gate_baseline_sha, nil)
    t.eq(count_calls("git fetch 'origin' 'dev'"), 0)
    t.eq(count_calls("git fetch 'origin' 'refs/pull/7/merge'"), 0)
    t.eq(count_calls("refs/remotes/'origin'/'dev'^{commit}"), 0)
    t.is_true(has_value(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.remove_labels, "fkst-dev:merge-ready"))
  end,

  test_merge_gate_marker_without_baseline_round_trips_nil = function()
    local event = merge_ready()
    local fix_version = core.fix_version_from_review_version(event.version)
    local request = core.build_merge_gate_fix_comment_request(
      "owner/repo",
      "42",
      event,
      fix_version,
      "rollup-red: test: COMPLETED/FAILURE",
      nil,
      event.source_ref
    )
    t.is_true(request.body:find("gate_baseline_sha", 1, true) == nil)
    local fix_fact = core.merge_gate_fix_fact({ request.body }, event.proposal_id, fix_version)
    t.eq(fix_fact.gate_baseline_sha, nil)
  end,

  test_merge_gate_fix_fact_selects_same_version_marker_by_event_baseline = function()
    local event = fixing({ gate_baseline_sha = "828df8d3" })
    local old_marker = core.merge_gate_marker(
      event.proposal_id,
      event.pr_number,
      event.version,
      event.review_proposal_id,
      event.review_dedup_key,
      event.reviewed_head_sha,
      "281c4f9e",
      "mergeable-conflicting"
    )
    local new_marker = core.merge_gate_marker(
      event.proposal_id,
      event.pr_number,
      event.version,
      event.review_proposal_id,
      event.review_dedup_key,
      event.reviewed_head_sha,
      event.gate_baseline_sha,
      "mergeable-conflicting"
    )

    local fact = core.merge_gate_fix_fact({ old_marker, new_marker }, event.proposal_id, event.version, {
      review_proposal_id = event.review_proposal_id,
      review_dedup_key = event.review_dedup_key,
      gate_baseline_sha = event.gate_baseline_sha,
      match_gate_baseline_sha = true,
    })
    t.eq(fact.gate_baseline_sha, event.gate_baseline_sha)

    local missing = core.merge_gate_fix_fact({ old_marker, new_marker }, event.proposal_id, event.version, {
      review_proposal_id = event.review_proposal_id,
      review_dedup_key = event.review_dedup_key,
      gate_baseline_sha = "feedface",
      match_gate_baseline_sha = true,
    })
    t.eq(missing, nil)
  end,

  test_fix_accepts_same_version_merge_gate_marker_matching_event_baseline = function()
    local event = fixing({
      gate_baseline_sha = "828df8d3",
      gate_failure_excerpt = "mergeable-conflicting",
    })
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local old_feedback = "github-devloop merge gate failed: mergeable-conflicting"
      .. "\n" .. core.merge_gate_marker(
        event.proposal_id,
        event.pr_number,
        event.version,
        event.review_proposal_id,
        event.review_dedup_key,
        event.reviewed_head_sha,
        "281c4f9e",
        "mergeable-conflicting"
      )
    local new_feedback = "github-devloop merge gate failed: mergeable-conflicting"
      .. "\n" .. core.merge_gate_marker(
        event.proposal_id,
        event.pr_number,
        event.version,
        event.review_proposal_id,
        event.review_dedup_key,
        event.reviewed_head_sha,
        event.gate_baseline_sha,
        "mergeable-conflicting"
      )
    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      old_feedback,
      new_feedback,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, event.reviewed_head_sha)
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    mock_existing_fix_worktree(branch, event.reviewed_head_sha, nil, {
      sha = event.gate_baseline_sha,
      exit_code = 0,
      stdout = "",
      stderr = "",
    })
    t.mock_command("git fetch 'origin' 'refs/pull/7/merge'", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("git rev-parse --verify FETCH_HEAD^{commit}", { stdout = event.gate_baseline_sha .. "\n", stderr = "", exit_code = 0 })
    mock_implement_codex(0, "fixed merge gate conflict")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      old_feedback,
      new_feedback,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, event.reviewed_head_sha)
    mock_git_push(branch)
    mock_pr_fix({ origin_marker }, branch, "feedface")

    local result = run_fix(event, opts("fix-same-version-merge-gate-baseline", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_reviewing").payload.version, core.next_fix_version(event.version))
    t.eq(count_calls("merge --no-edit '" .. event.gate_baseline_sha .. "'"), 1)
  end,

  test_synthetic_rollup_sha_still_cross_verifies_against_pr_merge_product = function()
    local event = merge_ready()
    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    -- Synthetic verify-branch coverage: live CheckRun rollup entries do not carry headSha.
    mock_pr_merge_rollup({ origin_marker }, '[{"name":"test","state":"COMPLETED","conclusion":"FAILURE","headSha":"bca321"}]', nil, nil, nil, nil, nil, nil, nil, nil, nil, "base999")
    t.mock_command("git fetch 'origin' 'refs/pull/7/merge'", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("git rev-parse --verify FETCH_HEAD^{commit}", { stdout = "bca321\n", stderr = "", exit_code = 0 })

    local result = run_merge(event, opts("merge-ci-red-synthetic-rollup-sha", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    local fixing_payload = find_raise(result.raises, "devloop_fixing").payload
    t.eq(fixing_payload.gate_baseline_sha, "bca321")
    local comment_body = find_raise(result.raises, "github-proxy.github_pr_comment_request").payload.body
    local fix_fact = core.merge_gate_fix_fact({ comment_body }, event.proposal_id, core.fix_version_from_review_version(event.version))
    t.eq(fix_fact.gate_baseline_sha, "bca321")
    t.eq(count_calls("git fetch 'origin' 'refs/pull/7/merge'"), 1)
  end,

  test_merge_ci_red_rejects_rollup_sha_that_is_not_pr_merge_product = function()
    local event = merge_ready()
    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    -- Synthetic verify-branch coverage: live CheckRun rollup entries do not carry headSha.
    mock_pr_merge_rollup({ origin_marker }, '[{"name":"test","state":"COMPLETED","conclusion":"FAILURE","headSha":"bca321"}]')
    t.mock_command("git fetch 'origin' 'refs/pull/7/merge'", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("git rev-parse --verify FETCH_HEAD^{commit}", { stdout = "feedface\n", stderr = "", exit_code = 0 })

    local result = run_merge(event, opts("merge-ci-red-sha-mismatch", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh pr merge"), 0)
  end,
}
