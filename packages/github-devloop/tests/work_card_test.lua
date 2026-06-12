local core = require("core")
local h = require("tests.devloop_helpers")
local t = fkst.test

local function source_ref()
  return {
    kind = "external",
    ref = "owner/repo#pr/7",
  }
end

return {
  test_work_card_comment_request_is_replaceable_view = function()
    local request = core.build_work_card_comment_request({
      kind = "pr",
      repo = "owner/repo",
      number = 7,
    }, {
      proposal_id = "github-devloop/issue/owner/repo/42",
      role = "fix",
      version = "v1/fix/2",
      round = 2,
      started_at = 1,
      finished_at = 91,
      outcome = "completed: pushed for re-review",
      gate_baseline_sha = "78ce5e97",
      last_stage = "review reject: missing guard",
      source_ref = source_ref(),
    })

    t.eq(request.schema, "github-proxy.v1")
    t.eq(request.pr_number, 7)
    t.is_true(request.body:find("Working: fix", 1, true) ~= nil)
    t.is_true(request.body:find("Duration: 1m 30s", 1, true) ~= nil)
    t.is_true(request.body:find("fkst:github-devloop:work-card:v1", 1, true) ~= nil)
    t.eq(request.body:find("fkst:github-devloop:state:v1", 1, true), nil)
    t.eq(request.replace_marker, core.work_card_marker("github-devloop/issue/owner/repo/42"))
  end,

  test_review_pr_raises_work_card_with_review_proposal = function()
    local event = h.reviewing()
    h.mock_bot_env()
    h.mock_write_env("1")
    h.mock_write_env("1")
    h.mock_write_env("1")
    h.mock_write_env("1")
    h.mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", event.version),
    }, {
      title = "Implement decision recorder",
      body = "Issue context",
    })
    h.mock_pr_origin_sequence({
      { head = "devloop-owner-repo-42-01HY", head_sha = "def456" },
    })
    local result = h.run_review_pr(event, h.opts("review-pr-work-card", {
      env = {
        FKST_GITHUB_WRITE = "1",
      },
    }))

    t.eq(result.exit_code, 0)
    local card = h.find_raise(result.raises, "github-proxy.github_pr_comment_request", function(payload)
      return tostring(payload.body or ""):find("fkst:github-devloop:work-card:v1", 1, true) ~= nil
    end)
    t.is_true(card ~= nil)
    t.eq(card.payload.replace_marker, core.work_card_marker("github-devloop/issue/owner/repo/42"))
    t.is_true(h.find_raise(result.raises, "consensus.proposal") ~= nil)
  end,
}
