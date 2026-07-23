local graph = require("testkit.graph")
local t = fkst.test

local function mock_dry_run()
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_comment_request_reaches_the_minimal_effect_package = function()
    mock_dry_run()

    local trace = graph.require_quiescent(graph.run({
      queue = "github-comment-effect.github_issue_comment_request",
      payload = {
        repo = "owner/repo",
        issue_number = 42,
        body = "dry-run integration coverage",
        dedup_key = "comment-effect/42/coverage",
        source_ref = { kind = "external", ref = "owner/repo#issue/42" },
      },
      source_ref = { kind = "external", reference = "owner/repo#issue/42" },
    }, { max_steps = 2 }))

    graph.assert_covers(trace, {
      "github-comment-effect.github_issue_comment_request -> github-comment-effect.github_comment",
    })
    local delivery = graph.require_delivery(trace, {
      queue = "github-comment-effect.github_issue_comment_request",
      consumer = "github-comment-effect.github_comment",
    })
    t.eq(delivery.exit_code, 0)
    t.eq(#(delivery.raises or {}), 0)
  end,

  test_issue_create_request_reaches_the_minimal_effect_package = function()
    mock_dry_run()

    local trace = graph.require_quiescent(graph.run({
      queue = "github-issue-effect.github_issue_create_request",
      payload = {
        schema = "github-proxy.issue-create.v1",
        repo = "owner/repo",
        title = "Dry-run integration coverage",
        body = "No external issue is created by this test.",
        dedup_key = "issue-effect/coverage/create",
        source_ref = { kind = "external", ref = "owner/repo#issue/42" },
      },
      source_ref = { kind = "external", reference = "owner/repo#issue/42" },
    }, { max_steps = 2 }))

    graph.assert_covers(trace, {
      "github-issue-effect.github_issue_create_request -> github-issue-effect.github_issue_create",
    })
    local delivery = graph.require_delivery(trace, {
      queue = "github-issue-effect.github_issue_create_request",
      consumer = "github-issue-effect.github_issue_create",
    })
    t.eq(delivery.exit_code, 0)
    t.eq(#(delivery.raises or {}), 0)
  end,
}
