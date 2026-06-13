local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local mock_pr_origin = h.mock_pr_origin
local find_raise = h.find_raise

local function find_label_raise(raises, target_kind)
  return find_raise(raises, "github-proxy.github_issue_label_request", function(payload)
    return tostring(payload.target_kind or "issue") == tostring(target_kind or "issue")
  end)
end

local function pr_opened_event(extra)
  local value = {
    schema = "github-proxy.pr-opened.v1",
    repo = "owner/repo",
    issue_number = 42,
    proposal_id = "github-devloop/issue/owner/repo/42",
    impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    pr_number = 7,
    branch = "devloop-owner-repo-42-01HY",
    head_sha = "def456",
    base_branch = "dev",
    dedup_key = "open-pr/github-devloop/issue/owner/repo/42/v1/devloop-owner-repo-42-01HY/opened/7",
    source_ref = {
      kind = "external",
      ref = "owner/repo#pr/7",
    },
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

return {
  test_pr_opened_direct_event_raises_reviewing = function()
    local event = pr_opened_event()
    t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
      stdout = "dev",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    mock_pr_origin({
      core.pr_origin_marker(event.proposal_id, "42", event.branch, event.impl_version, event.base_branch),
      core.state_marker(event.proposal_id, "pr-open", event.impl_version),
    }, event.branch, event.head_sha)

    local result = t.run_department("departments/observe_pr/main.lua", {
      queue = "github-proxy.github_pr_opened",
      payload = event,
    }, opts("pr-opened-direct-reviewing"))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 4)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local pr_label_raise = find_label_raise(result.raises, "pr")
    local reviewing_raise = find_raise(result.raises, "devloop_reviewing")
    t.eq(comment_raise.payload.pr_number, 7)
    t.eq(pr_label_raise.payload.target_number, 7)
    t.eq(pr_label_raise.payload.expected_state, "reviewing")
    t.eq(pr_label_raise.payload.expected_version, event.impl_version)
    t.eq(reviewing_raise.payload.proposal_id, event.proposal_id)
    t.eq(reviewing_raise.payload.pr_number, 7)
    t.eq(reviewing_raise.payload.version, event.impl_version)
    t.eq(reviewing_raise.payload.source_ref.ref, "owner/repo#pr/7")
  end,
}
