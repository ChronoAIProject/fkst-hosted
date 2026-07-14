local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core

return {
  test_namespaced_review_meta_with_same_version_fixing_predecessor_retries = function()
    local event = h.review_meta_event()
    h.mock_issue_review_meta({}, {
      core.state_marker(event.proposal_id, "fixing", event.version),
    })
    h.mock_default_issue_claim()
    h.mock_pr_origin(nil, "devloop-owner-repo-42-01HY", "def456", "OPEN", "dev")

    local result = h.run_department("departments/review_meta/main.lua", {
      queue = "github-devloop-pr.devloop_review_meta",
      payload = event,
    }, h.opts("review-meta-namespaced-fixing-predecessor"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.is_true(tostring(result.error or ""):find("review%-meta state marker not yet visible; retrying") ~= nil)
  end,
}
