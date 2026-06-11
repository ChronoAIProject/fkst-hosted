local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local issue = h.issue
local reviewing = h.reviewing
local review_reached = h.review_reached
local fixing = h.fixing
local merge_ready = h.merge_ready
local run_observe = h.run_observe
local run_review_result = h.run_review_result
local run_fix = h.run_fix
local run_merge = h.run_merge
local mock_issue_state = h.mock_issue_state
local mock_issue_result = h.mock_issue_result
local mock_issue_fix_for_event = h.mock_issue_fix_for_event
local mock_issue_merge = h.mock_issue_merge
local mock_pr_origin = h.mock_pr_origin
local mock_pr_fix = h.mock_pr_fix
local mock_pr_merge = h.mock_pr_merge
local mock_pr_merge_rollup = h.mock_pr_merge_rollup
local mock_implement_codex = h.mock_implement_codex
local mock_existing_fix_worktree = h.mock_existing_fix_worktree
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local mock_git_push = h.mock_git_push
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local merge_comments = h.merge_comments
local find_raise = h.find_raise
local count_calls = h.count_calls

local function find_pr_comment_with(raises, needle)
  for _, raised in ipairs(raises or {}) do
    if raised.queue == "github-proxy.github_pr_comment_request"
      and tostring((raised.payload or {}).body or ""):find(needle, 1, true) ~= nil then
      return raised
    end
  end
  return nil
end

local function review_origin_marker(version, head_sha)
  return core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", version, "dev")
end

local function mock_issue_result_view(labels, comments)
  local rendered_labels = {}
  for _, label in ipairs(labels or {}) do
    table.insert(rendered_labels, string.format('{"name":"%s"}', h.json_string(label)))
  end
  local rendered_comments = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered_comments, h.render_comment(comment))
  end
  t.mock_command("--json labels,comments", {
    stdout = string.format('{"labels":[%s],"comments":[%s]}\n', table.concat(rendered_labels, ","), table.concat(rendered_comments, ",")),
    stderr = "",
    exit_code = 0,
  })
end

local function reject_comment(fix)
  return core.build_review_result_comment_request(
    "owner/repo",
    "42",
    fix.proposal_id,
    fix.version,
    {
      proposal_id = fix.review_proposal_id,
      decision = "reject",
      body = "Reject because tests failed.",
      blocking_gap = "missing regression guard",
      dedup_key = fix.review_dedup_key,
      source_ref = fix.source_ref,
    },
    fix.source_ref
  ).body
end

return {
  test_fix_direct_raise_and_poll_recovery_reraise_same_reviewing_kickoff = function()
    local event = fixing()
    local branch = "devloop-owner-repo-42-01HY"
    local origin_marker = review_origin_marker(event.version)
    local feedback = reject_comment(event)
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      feedback,
    }, branch, event.version)
    mock_pr_fix({ origin_marker, feedback }, branch, event.reviewed_head_sha)
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    mock_existing_fix_worktree(branch, event.reviewed_head_sha)
    mock_implement_codex(0, "fixed")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      feedback,
    }, branch, event.version)
    mock_pr_fix({ origin_marker, feedback }, branch, event.reviewed_head_sha)
    mock_git_push(branch)
    mock_pr_fix({ origin_marker, feedback }, branch, "feedface")

    local direct = run_fix(event, opts("internal-chain-fix-direct", { FKST_GITHUB_WRITE = "1" }))
    t.eq(direct.exit_code, 0)
    local direct_reviewing = find_raise(direct.raises, "devloop_reviewing")
    t.eq(direct_reviewing.payload.version, core.next_fix_version(event.version))
    t.eq(direct_reviewing.payload.pr_number, event.pr_number)

    local impl_version = core._strip_latest_fix_version_suffix(event.version)
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:fixing" }, "OPEN", {
      core.pr_link_marker(event.proposal_id, event.pr_number, branch, impl_version, "dev"),
      core.state_marker(event.proposal_id, "fixing", event.version),
    })
    mock_pr_origin({
      core.pr_origin_marker(event.proposal_id, "42", branch, impl_version, "dev"),
      core.state_marker(event.proposal_id, "fixing", event.version),
    })
    local recovered = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:fixing" } }), opts("internal-chain-fix-recovery"))
    t.eq(recovered.exit_code, 0)
    local recovered_reviewing = find_raise(recovered.raises, "devloop_reviewing")
    t.eq(recovered_reviewing.payload.dedup_key, direct_reviewing.payload.dedup_key)
    t.eq(recovered_reviewing.payload.version, direct_reviewing.payload.version)
  end,

  test_review_result_direct_raise_and_poll_recovery_cover_merge_ready_and_fixing = function()
    local impl_version = reviewing().version
    local approve = review_reached()
    mock_bot_env()
    mock_pr_origin({ review_origin_marker(impl_version) })
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })
    local approved = run_review_result(approve, opts("internal-chain-review-approve-direct"))
    t.eq(approved.exit_code, 0)
    local direct_merge = find_raise(approved.raises, "devloop_merge_ready")
    t.eq(direct_merge.payload.schema, "github-devloop.merge-ready.v1")

    mock_pr_origin({ review_origin_marker(impl_version) })
    h.set_pr_phase_comments({ "fkst-dev:merge-ready" }, merge_comments(direct_merge.payload))
    local recovered_merge = h.run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    }, opts("internal-chain-review-approve-recovery"))
    t.eq(recovered_merge.exit_code, 0)
    t.eq(find_raise(recovered_merge.raises, "devloop_merge_ready").payload.dedup_key, direct_merge.payload.dedup_key)

    local reject = review_reached({
      decision = "reject",
      body = "Review consensus rejects the diff.",
      blocking_gap = "missing regression guard",
    })
    mock_bot_env()
    mock_pr_origin({ review_origin_marker(impl_version) })
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })
    local rejected = run_review_result(reject, opts("internal-chain-review-reject-direct"))
    t.eq(rejected.exit_code, 0)
    local direct_fix = find_raise(rejected.raises, "devloop_fixing")
    t.eq(direct_fix.payload.schema, "github-devloop.fixing.v1")

    local reject_fact = find_pr_comment_with(rejected.raises, "fkst:github-devloop:review-result:v1").payload.body
    mock_pr_origin({
      review_origin_marker(impl_version),
      core.state_marker("github-devloop/issue/owner/repo/42", "fixing", direct_fix.payload.version),
      reject_fact,
    })
    mock_issue_result_view({ "fkst-dev:fixing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "fixing", direct_fix.payload.version),
      reject_fact,
    })
    local recovered_fix = h.run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:04Z",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    }, opts("internal-chain-review-reject-recovery"))
    t.eq(recovered_fix.exit_code, 0)
    t.eq(find_raise(recovered_fix.raises, "devloop_fixing").payload.dedup_key, direct_fix.payload.dedup_key)
  end,

  test_merge_direct_cascade_and_poll_recovery_cover_terminal_and_repair_paths = function()
    local event = merge_ready()
    local origin_marker = review_origin_marker(event.version)
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_pr_merge_rollup(merge_comments(event), '[{"name":"test","state":"COMPLETED","conclusion":"FAILURE"}]')

    local red = run_merge(event, opts("internal-chain-merge-red-direct", { FKST_GITHUB_WRITE = "1" }))
    t.eq(red.exit_code, 0)
    local direct_fix = find_raise(red.raises, "devloop_fixing")
    t.eq(direct_fix.payload.schema, "github-devloop.fixing.v1")
    t.eq(count_calls("gh pr merge"), 0)

    local merge_gate_comment = find_raise(red.raises, "github-proxy.github_pr_comment_request").payload.body
    mock_pr_origin({ origin_marker, merge_gate_comment })
    mock_issue_result_view({ "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", direct_fix.payload.version),
      merge_gate_comment,
    })
    local recovered_fix = h.run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:05Z",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    }, opts("internal-chain-merge-red-recovery"))
    t.eq(recovered_fix.exit_code, 0)
    t.eq(find_raise(recovered_fix.raises, "devloop_fixing").payload.dedup_key, direct_fix.payload.dedup_key)

    mock_bot_env()
    mock_write_env("1")
    mock_pr_merge(merge_comments(event), "devloop-owner-repo-42-01HY", "def456", "MERGED", "owner/repo", false, "MERGEABLE", "CLEAN", "COMPLETED", "SUCCESS", "2026-06-03T02:03:04Z")
    mock_write_env("1")
    h.mock_issue_close()
    local terminal = run_merge(event, opts("internal-chain-merge-terminal-recovery", { FKST_GITHUB_WRITE = "1" }))
    t.eq(terminal.exit_code, 0)
    t.eq(find_raise(terminal.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:merged")
    t.eq(find_raise(terminal.raises, "devloop_fixing"), nil)
    t.eq(find_raise(terminal.raises, "devloop_reviewing"), nil)
  end,
}
