local core = require("core")
local t = fkst.test

local function source_ref()
  return {
    kind = "external",
    ref = "owner/repo#issue/42",
  }
end

local function issue(extra)
  local value = {
    schema = "github-proxy.v1",
    type = "issue",
    repo = "owner/repo",
    number = 42,
    title = "Implement decision recorder",
    url = "https://github.example/owner/repo/issues/42",
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    labels = { "fkst-dev:enabled" },
    dedup_key = "owner/repo#issue#42@2026-06-03T01:02:03Z",
    source_ref = source_ref(),
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function reached(extra)
  local value = {
    schema = "consensus.consensus_reached.v1",
    proposal_id = "github-devloop/issue/owner/repo/42",
    decision = "approve",
    body = "All angles approve.",
    dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    source_ref = source_ref(),
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

return {
  test_opt_in_detection = function()
    t.eq(core.is_opted_in({ "fkst-dev:enabled" }), true)
    t.eq(core.is_opted_in({ "bug" }), false)
    t.eq(core.is_opted_in({ "fkst-dev:enabled", "fkst-dev:thinking" }), false)
    t.eq(core.is_opted_in({ "fkst-dev:enabled", "fkst-dev:ready" }), false)
    t.eq(core.is_opted_in({ "fkst-dev:enabled", "fkst-dev:blocked" }), false)
  end,

  test_proposal_id_round_trip = function()
    local id = core.proposal_id("owner/repo", 42)
    t.eq(id, "github-devloop/issue/owner/repo/42")
    local repo, issue_number = core.parse_proposal_id(id)
    t.eq(repo, "owner/repo")
    t.eq(issue_number, "42")
    t.eq(core.issue_ref_round_trips("owner/repo", 42), true)
    t.is_nil(core.parse_proposal_id("autochrono/issue/owner/repo/42"))
  end,

  test_bounded_body = function()
    t.eq(core.bounded_body("hello"), "hello")
    t.eq(core.bounded_body(""), "(empty issue body)")
    local bounded = core.bounded_body(string.rep("x", core.max_body_len() + 10))
    t.eq(#bounded, core.max_body_len())
  end,

  test_build_proposal = function()
    local proposal = core.build_proposal(issue(), "Issue body")
    t.eq(proposal.schema, "consensus.proposal.v1")
    t.eq(proposal.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(proposal.title, "Implement decision recorder")
    t.eq(proposal.body, "Issue body")
    t.eq(proposal.dedup_key, "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z")
    t.eq(proposal.source_ref.ref, "owner/repo#issue/42")
    t.eq(core.validate_proposal(proposal), true)
  end,

  test_marker_label_and_comment_builders = function()
    local marker = core.result_marker(
      "github-devloop/issue/owner/repo/42",
      "approve",
      "consensus:github-devloop/issue/owner/repo/42/v1"
    )
    t.eq(
      marker,
      '<!-- fkst:github-devloop:result:v1 proposal="github-devloop/issue/owner/repo/42" decision="approve" dedup="consensus:github-devloop/issue/owner/repo/42/v1" -->'
    )

    local label = core.build_result_label_request("owner/repo", "42", reached())
    t.eq(label.schema, "github-proxy.label.v1")
    t.eq(label.add_labels[1], "fkst-dev:ready")
    t.eq(label.remove_labels[1], "fkst-dev:thinking")
    t.eq(label.issue_number, "42")

    local rejected = core.build_result_label_request("owner/repo", "42", reached({ decision = "reject" }))
    t.eq(rejected.add_labels[1], "fkst-dev:blocked")

    local completed = reached()
    local comment = core.build_result_comment_request("owner/repo", "42", completed)
    t.eq(comment.schema, "github-proxy.v1")
    t.eq(comment.issue_number, "42")
    t.is_true(comment.body:find("github-devloop decision: approve", 1, true) ~= nil)
    t.is_true(comment.body:find('fkst:github-devloop:result:v1 proposal="github-devloop/issue/owner/repo/42"', 1, true) ~= nil)
    local comment_version = tostring(completed.dedup_key):gsub(":", "-")
    t.eq(
      comment.dedup_key,
      tostring(completed.proposal_id) .. "/comment/" .. tostring(completed.decision) .. "/" .. comment_version
    )
  end,

  test_comment_dedup_key_includes_consensus_version = function()
    local first = reached({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/v1",
    })
    local second = reached({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/v2",
    })

    local first_comment = core.build_result_comment_request("owner/repo", "42", first)
    local second_comment = core.build_result_comment_request("owner/repo", "42", second)

    t.eq(first_comment.dedup_key, "github-devloop/issue/owner/repo/42/comment/approve/consensus-github-devloop/issue/owner/repo/42/v1")
    t.eq(second_comment.dedup_key, "github-devloop/issue/owner/repo/42/comment/approve/consensus-github-devloop/issue/owner/repo/42/v2")
    t.eq(first_comment.dedup_key ~= second_comment.dedup_key, true)
  end,

  test_gh_issue_view_body_command_and_parse = function()
    t.eq(
      core.gh_issue_view_body_cmd("owner/repo", 42),
      "gh issue view '42' --repo 'owner/repo' --json body"
    )
    t.eq(
      core.gh_issue_view_state_cmd("owner/repo", 42),
      "gh issue view '42' --repo 'owner/repo' --json labels,state"
    )
    t.eq(
      core.gh_issue_view_result_cmd("owner/repo", 42),
      "gh issue view '42' --repo 'owner/repo' --json labels,comments"
    )
    t.eq(core.parse_issue_view_body('{"body":"Hello"}'), "Hello")

    local state = core.parse_issue_view_state('{"state":"OPEN","labels":[{"name":"fkst-dev:enabled"}]}')
    t.eq(state.state, "OPEN")
    t.eq(state.labels[1], "fkst-dev:enabled")

    local proposal_id = "github-devloop/issue/owner/repo/42"
    local decision = "approve"
    local dedup_key = "consensus:github-devloop/issue/owner/repo/42/v1"
    local result = core.parse_issue_view_result(
      '{"labels":["fkst-dev:ready"],"comments":[{"body":"'
        .. core.result_marker(proposal_id, decision, dedup_key):gsub('"', '\\"')
        .. '"}]}'
    )
    t.eq(core.has_terminal_label(result.labels), true)
    t.eq(core.has_result_marker(result.comments, proposal_id, decision, dedup_key), true)
  end,
}
