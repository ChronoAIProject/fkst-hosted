local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local run_observe_pr = h.run_observe_pr
local run_observe = h.run_observe
local find_raise = h.find_raise
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local issue_proposal_id = "github-devloop/issue/owner/repo/42"
local pr_proposal_id = "github-devloop/pr/owner/repo/7"
local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local branch = "devloop-owner-repo-42-01HY"
local head_sha = "def456"

local function observe_pr_event()
  return {
    schema = "github-proxy.v1",
    type = "pr",
    repo = "owner/repo",
    number = 7,
    state = "OPEN",
    updated_at = "2026-06-03T02:03:04Z",
    dedup_key = "owner/repo#pr#7@2026-06-03T02:03:04Z",
    source_ref = {
      kind = "external",
      ref = "owner/repo#pr/7",
    },
  }
end

local function observe_issue_event()
  return {
    schema = "github-proxy.v1",
    type = "issue",
    repo = "owner/repo",
    number = 42,
    title = "Implement decision recorder",
    state = "OPEN",
    updated_at = "2026-06-03T02:03:04Z",
    labels = { "fkst-dev:enabled", "fkst-dev:awaiting-pr" },
    dedup_key = "owner/repo#issue#42@2026-06-03T02:03:04Z",
    source_ref = core.issue_source_ref("owner/repo", 42),
  }
end

local function awaiting_pr_comments()
  return {
    core.state_marker(issue_proposal_id, "awaiting-pr", impl_version),
    core.pr_delegation_marker(issue_proposal_id, pr_proposal_id, 7, impl_version, "g1"),
  }
end

local function mock_issue_at_awaiting_pr(selector)
  local issue = {
    repo = "owner/repo",
    number = 42,
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
    labels = { "fkst-dev:enabled", "fkst-dev:awaiting-pr" },
    comments = awaiting_pr_comments(),
  }
  entity_read_mocks.mock_issue_read_forms(t, issue)
  entity_read_mocks.mock_issue_view_selector(t, issue, selector or "assignees,author")
end

local function mock_pr_with_comments(comments)
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = "owner/repo",
    number = 7,
    comments = comments,
    head = branch,
    head_sha = head_sha,
    base_branch = "dev",
    state = "OPEN",
  }, entity_read_mocks.pr_origin_selector)
end

local function find_exact_raise(raises, queue, predicate)
  for _, raised in ipairs(raises or {}) do
    if raised.queue == queue and (predicate == nil or predicate(raised.payload or {}, raised)) then
      return raised
    end
  end
  return nil
end

local function pr_open_comments()
  return {
    core.pr_origin_marker(issue_proposal_id, 42, branch, impl_version, "dev"),
    core.state_marker(issue_proposal_id, "pr-open", impl_version),
  }
end

return {
  test_delegated_pr_open_is_reviewed_while_parent_waits_at_awaiting_pr = function()
    mock_issue_at_awaiting_pr()
    mock_pr_with_comments(pr_open_comments())

    local result = run_observe_pr(observe_pr_event(), opts("awaiting-pr-observe-pr-review"))

    t.eq(result.exit_code, 0)
    local review_comment = find_raise(result.raises, "github-proxy.github_pr_comment_request", function(payload)
      return tostring(payload.body or ""):find('state="reviewing"', 1, true) ~= nil
        and tostring(payload.body or ""):find('proposal="' .. issue_proposal_id .. '"', 1, true) ~= nil
    end)
    t.is_true(review_comment ~= nil)
    t.is_true(review_comment.payload.handoff ~= nil)
    t.eq(review_comment.payload.handoff.proposal_id, issue_proposal_id)
    t.is_true(find_raise(result.raises, "devloop_reviewing") ~= nil)
  end,

  test_awaiting_pr_parent_replay_noops_while_child_pr_is_nonterminal = function()
    mock_issue_at_awaiting_pr("number,title,body,comments,labels,state,updatedAt,assignees,author")
    mock_pr_with_comments({
      core.pr_origin_marker(issue_proposal_id, 42, branch, impl_version, "dev"),
      core.state_marker(issue_proposal_id, "reviewing", impl_version),
    })

    local result = run_observe(observe_issue_event(), opts("awaiting-pr-observe-issue-child-nonterminal"))

    t.eq(result.exit_code, 0)
    t.eq(find_exact_raise(result.raises, "github-proxy.github_issue_comment_request", function(payload)
      local body = tostring(payload.body or "")
      return body:find("resumed parent issue", 1, true) ~= nil
        or body:find('state="merged"', 1, true) ~= nil
        or body:find('state="ready"', 1, true) ~= nil
        or body:find('state="blocked"', 1, true) ~= nil
    end), nil)
  end,
}
