local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local find_raise = h.find_raise

local function mock_repo_env()
  h.mock_bot_env()
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_ROLLUP_MERGE"', { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "", stderr = "", exit_code = 0 })
end

return {
  test_scan_other_authored_unassigned_issue_raises_fork_request_only = function()
    mock_repo_env()
    t.mock_command(core.gh_issue_list_intake_cmd("owner/repo", 100), {
      stdout = '[{"number":42,"title":"External request","body":"","createdAt":"2026-06-03T01:00:00Z","updatedAt":"2026-06-03T01:02:03Z","labels":[],"assignees":[],"author":{"login":"human"}}]\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_view_intake_scan_cmd("owner/repo", "42"), {
      stdout = '{"title":"External request","state":"OPEN","labels":[],"comments":[],"assignees":[],"author":{"login":"human"}}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = t.run_department("departments/intake_scan/main.lua", {
      queue = "devloop_intake_tick",
      payload = { schema = "github-devloop.intake-tick.v1" },
    }, opts("fork-intake-scan-other-author"))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local request = find_raise(result.raises, "github-proxy.github_issue_create_request").payload
    t.eq(request.assignees[1], "fkst-test-bot")
    t.eq(request.parent_comment_target.issue_number, 42)
    t.eq(request.post_create_blocked_by.blocked_issue_number, 42)
    t.eq(find_raise(result.raises, "devloop_intake_candidate"), nil)
  end,
}
