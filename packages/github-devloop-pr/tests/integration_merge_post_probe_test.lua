local h = require("tests.devloop_helpers")
local autonomy_ledger = require("devloop.autonomy_ledger")
local m_builders = require("devloop.markers.builders")
local t = h.t
local core = h.core
local opts = h.opts
local merge_ready = h.merge_ready
local run_merge = h.run_merge
local mock_bot_env = h.mock_bot_env
local mock_write_env = h.mock_write_env
local mock_issue_merge = h.mock_issue_merge
local merge_comments = h.merge_comments
local mock_pr_merge = h.mock_pr_merge
local mock_merging_comment = h.mock_merging_comment
local mock_pr_merge_command = h.mock_pr_merge_command
local mock_issue_close = h.mock_issue_close
local find_raise = h.find_raise

return {
  test_merge_records_failed_post_merge_probe_without_silent_pass = function()
    local event = merge_ready()
    local origin_marker = m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge({ origin_marker })
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_write_env("1")
    mock_pr_merge({ origin_marker })
    mock_merging_comment()
    mock_pr_merge_command()
    mock_pr_merge({ origin_marker }, "devloop-owner-repo-42-01HY", "def456", "MERGED", "owner/repo", false, "MERGEABLE", "CLEAN", "COMPLETED", "FAILURE", "2026-06-03T02:03:04Z")
    mock_issue_close()

    local result = run_merge(event, opts("merge-post-merge-probe-red", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local avm = autonomy_ledger.autonomy_result_fact(core, { comment_raise.payload.body }, event.proposal_id, event.pr_number, event.version, event.reviewed_head_sha)
    t.eq(avm.gates.post_merge_probe, "fail")
    t.eq(avm.valid_autonomous_merge, "false")
    t.is_true(comment_raise.payload.body:find('post_merge_probe_green="fail"', 1, true) ~= nil)
  end,
}
