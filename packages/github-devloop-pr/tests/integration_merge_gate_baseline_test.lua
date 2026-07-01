local devloop_base = require("devloop.base")
local requests_review = require("devloop.requests.review")
local h = require("tests.devloop_helpers")
local payloads_builders = require("devloop.payloads.builders")
local m_facts = require("devloop.markers.facts")
local m_builders = require("devloop.markers.builders")
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
local find_causal_raise = h.find_causal_raise
local check_runs_cmd = "gh api 'repos/owner/repo/commits/def456/check-runs'"

local function mock_failing_required_check_runs()
  t.mock_command(check_runs_cmd, {
    stdout = '{"total_count":1,"check_runs":[{"name":"test","status":"completed","conclusion":"failure","head_sha":"def456"}]}\n',
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_merge_ci_red_without_rollup_sha_uses_pr_base_baseline = function()
    local event = merge_ready()
    local origin_marker = m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge_rollup({ origin_marker }, '[{"__typename":"CheckRun","completedAt":"2026-06-03T02:04:04Z","conclusion":"FAILURE","detailsUrl":"https://example.invalid/checks/test","name":"test","startedAt":"2026-06-03T02:03:04Z","status":"COMPLETED","workflowName":"ci","headSha":"def456"}]', nil, nil, nil, nil, nil, nil, nil, nil, nil, "ba5e9999")
    mock_failing_required_check_runs()

    local result = run_merge(event, opts("merge-ci-red", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(count_calls("gh pr merge"), 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:fixing")
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local fixing_handoff = comment_raise.payload.handoff
    t.eq(fixing_handoff.kind, "github-devloop.fixing")
    t.eq(fixing_handoff.blocking_gap, nil)
    t.mock_command("gh api --method GET 'repos/owner/repo/issues/comments/IC_merge_ci_red_fixing_1'", {
      stdout = '{"body":"' .. h.json_string(core.state_marker(fixing_handoff.proposal_id, "fixing", fixing_handoff.version)) .. '","user":{"login":"fkst-test-bot"}}\n',
      stderr = "",
      exit_code = 0,
    })
    local handoff_result = t.run_department("departments/comment_handoff/main.lua", {
      queue = "github-proxy.github_comment_written",
      payload = {
        schema = "github-proxy.comment-written.v1",
        repo = comment_raise.payload.repo,
        target = "pr",
        pr_number = comment_raise.payload.pr_number,
        comment_id = "IC_merge_ci_red_fixing_1",
        request_dedup_key = comment_raise.payload.dedup_key,
        dedup_key = comment_raise.payload.dedup_key .. "/written/IC_merge_ci_red_fixing_1",
        source_ref = comment_raise.payload.source_ref,
        handoff = fixing_handoff,
      },
    }, opts("merge-ci-red-fixing-comment-handoff"))
    t.eq(handoff_result.exit_code, 0)
    local fixing_payload = find_raise(handoff_result.raises, "devloop_fixing").payload
    t.eq(fixing_payload.schema, "github-devloop.fixing.v1")
    t.eq(fixing_payload.gate_baseline_sha, "ba5e9999")
    t.eq(fixing_payload.gate_failure_excerpt, "own-ci-red")
    t.eq(fixing_payload.blocking_gap, nil)
    local comment_body = comment_raise.payload.body
    t.is_true(comment_body:find("fkst:github-devloop:merge-gate:v1", 1, true) ~= nil)
    t.is_true(comment_body:find("gate_baseline_sha", 1, true) ~= nil)
    t.is_true(comment_body:find("own-ci-red", 1, true) ~= nil)
    t.is_true(comment_body:find("Reproduce locally with `scripts/run.sh test`", 1, true) ~= nil)
    local fix_fact = m_facts.merge_gate_fix_fact(core, { comment_body }, event.proposal_id, core.fix_version_from_review_version(event.version))
    t.is_true(fix_fact.review_reason:find("own-ci-red", 1, true) ~= nil)
    t.eq(fix_fact.gate_baseline_sha, "ba5e9999")
    t.eq(count_calls("git fetch 'origin' 'dev'"), 0)
    t.eq(count_calls("git fetch 'origin' 'refs/pull/7/merge'"), 0)
    t.eq(count_calls("refs/remotes/'origin'/'dev'^{commit}"), 0)
    t.is_true(has_value(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.remove_labels, "fkst-dev:merge-ready"))
  end,

  test_merge_gate_marker_without_baseline_round_trips_nil = function()
    local event = merge_ready()
    local fix_version = core.fix_version_from_review_version(event.version)
    local request = requests_review.build_merge_gate_fix_comment_request(core,
      "owner/repo",
      "42",
      event,
      fix_version,
      "rollup-red: test: COMPLETED/FAILURE",
      nil,
      event.source_ref
    )
    t.is_true(request.body:find("gate_baseline_sha", 1, true) == nil)
    local fix_fact = m_facts.merge_gate_fix_fact(core, { request.body }, event.proposal_id, fix_version)
    t.eq(fix_fact.gate_baseline_sha, nil)
  end,

  test_merge_gate_fix_fact_selects_same_version_marker_by_event_baseline = function()
    local event = fixing({ gate_baseline_sha = "828df8d3" })
    local old_marker = m_builders.merge_gate_marker(core, 
      event.proposal_id,
      event.pr_number,
      event.version,
      event.review_proposal_id,
      event.review_dedup_key,
      event.reviewed_head_sha,
      "281c4f9e",
      "mergeable-conflicting"
    )
    local new_marker = m_builders.merge_gate_marker(core, 
      event.proposal_id,
      event.pr_number,
      event.version,
      event.review_proposal_id,
      event.review_dedup_key,
      event.reviewed_head_sha,
      event.gate_baseline_sha,
      "mergeable-conflicting"
    )

    local fact = m_facts.merge_gate_fix_fact(core, { old_marker, new_marker }, event.proposal_id, event.version, {
      review_proposal_id = event.review_proposal_id,
      review_dedup_key = event.review_dedup_key,
      gate_baseline_sha = event.gate_baseline_sha,
      match_gate_baseline_sha = true,
    })
    t.eq(fact.gate_baseline_sha, event.gate_baseline_sha)

    local missing = m_facts.merge_gate_fix_fact(core, { old_marker, new_marker }, event.proposal_id, event.version, {
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
    local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
    local old_feedback = "github-devloop merge gate failed: mergeable-conflicting"
      .. "\n" .. m_builders.merge_gate_marker(core, 
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
      .. "\n" .. m_builders.merge_gate_marker(core, 
        event.proposal_id,
        event.pr_number,
        event.version,
        event.review_proposal_id,
        event.review_dedup_key,
        event.reviewed_head_sha,
        event.gate_baseline_sha,
        "mergeable-conflicting"
      )
    local origin_marker = m_builders.pr_origin_marker(core, event.proposal_id, "42", branch, event.version, "dev")
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
    t.eq(find_causal_raise(result, "devloop_reviewing").payload.version, core.next_fix_version(event.version))
    t.eq(count_calls("merge --no-edit '" .. event.gate_baseline_sha .. "'"), 1)
    t.eq(count_calls("git fetch 'origin' 'refs/pull/7/merge'"), 0)
  end,

  test_corrected_merge_gate_replay_dedup_reaches_fix_after_nil_baseline_predecessor = function()
    local event = fixing({
      gate_baseline_sha = "828df8d3",
      gate_failure_excerpt = "mergeable-conflicting",
    })
    local defective = payloads_builders.build_replayed_fixing_payload(core, {
      proposal_id = event.proposal_id,
      impl_version = event.version,
    }, event.pr_number, {
      review_proposal_id = event.review_proposal_id,
      review_dedup_key = event.review_dedup_key,
      reviewed_head_sha = event.reviewed_head_sha,
      blocking_gap = "mergeable-conflicting",
    }, event.source_ref)
    local corrected = payloads_builders.build_replayed_fixing_payload(core, {
      proposal_id = event.proposal_id,
      impl_version = event.version,
    }, event.pr_number, {
      review_proposal_id = event.review_proposal_id,
      review_dedup_key = event.review_dedup_key,
      reviewed_head_sha = event.reviewed_head_sha,
      blocking_gap = "mergeable-conflicting",
      gate_baseline_sha = event.gate_baseline_sha,
      review_reason = "mergeable-conflicting",
    }, event.source_ref)
    local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
    local feedback = "github-devloop merge gate failed: mergeable-conflicting"
      .. "\n" .. m_builders.merge_gate_marker(core, 
        event.proposal_id,
        event.pr_number,
        event.version,
        event.review_proposal_id,
        event.review_dedup_key,
        event.reviewed_head_sha,
        event.gate_baseline_sha,
        "mergeable-conflicting"
      )
    local origin_marker = m_builders.pr_origin_marker(core, event.proposal_id, "42", branch, event.version, "dev")

    t.is_true(defective.dedup_key ~= corrected.dedup_key)
    t.is_true(defective.dedup_key:find("/nobase/nopred/" .. event.reviewed_head_sha, 1, true) ~= nil)
    t.is_true(corrected.dedup_key:find("/" .. event.gate_baseline_sha .. "/nopred/" .. event.reviewed_head_sha, 1, true) ~= nil)
    t.eq(corrected.gate_baseline_sha, event.gate_baseline_sha)

    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(corrected, { "fkst-dev:fixing" }, {
      core.state_marker(corrected.proposal_id, "fixing", corrected.version),
      feedback,
    }, branch, corrected.version)
    mock_pr_fix({ origin_marker }, branch, corrected.reviewed_head_sha)
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    mock_existing_fix_worktree(branch, corrected.reviewed_head_sha, nil, {
      sha = corrected.gate_baseline_sha,
      exit_code = 0,
      stdout = "",
      stderr = "",
    })
    mock_implement_codex(0, "fixed corrected replay")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_issue_fix_for_event(corrected, { "fkst-dev:fixing" }, {
      core.state_marker(corrected.proposal_id, "fixing", corrected.version),
      feedback,
    }, branch, corrected.version)
    mock_pr_fix({ origin_marker }, branch, corrected.reviewed_head_sha)
    mock_git_push(branch)
    mock_pr_fix({ origin_marker }, branch, "feedface")

    local result = run_fix(corrected, opts("fix-corrected-replay-after-nil-baseline", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(find_causal_raise(result, "devloop_reviewing").payload.version, core.next_fix_version(corrected.version))
    t.eq(count_calls("merge --no-edit '" .. corrected.gate_baseline_sha .. "'"), 1)
    t.eq(count_calls("git fetch 'origin' 'refs/pull/7/merge'"), 0)
  end,

  test_synthetic_rollup_sha_no_longer_drives_pr_fixing = function()
    local event = merge_ready()
    local origin_marker = m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    -- Synthetic verify-branch coverage: live CheckRun rollup entries do not carry headSha.
    mock_pr_merge_rollup({ origin_marker }, '[{"name":"test","state":"COMPLETED","conclusion":"FAILURE","headSha":"bca321"}]', nil, nil, nil, nil, nil, nil, nil, nil, nil, "base999")
    t.mock_command(check_runs_cmd, {
      stdout = '{"total_count":1,"check_runs":[{"name":"test","status":"completed","conclusion":"success","head_sha":"def456"}]}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = run_merge(event, opts("merge-ci-red-synthetic-rollup-sha", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 1)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    local comment_body = find_raise(result.raises, "github-proxy.github_pr_comment_request").payload.body
    t.is_true(comment_body:find("fkst:github-devloop:merge-gate-wait:v1", 1, true) ~= nil)
    t.eq(count_calls("git fetch 'origin' 'refs/pull/7/merge'"), 0)
  end,

  test_merge_ci_red_ignores_rollup_sha_that_is_not_pr_head = function()
    local event = merge_ready()
    local origin_marker = m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    -- Synthetic verify-branch coverage: live CheckRun rollup entries do not carry headSha.
    mock_pr_merge_rollup({ origin_marker }, '[{"name":"test","state":"COMPLETED","conclusion":"FAILURE","headSha":"bca321"}]')
    t.mock_command(check_runs_cmd, {
      stdout = '{"total_count":1,"check_runs":[{"name":"test","status":"completed","conclusion":"success","head_sha":"def456"}]}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = run_merge(event, opts("merge-ci-red-sha-mismatch", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 1)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    t.eq(count_calls("gh pr merge"), 0)
  end,
}
