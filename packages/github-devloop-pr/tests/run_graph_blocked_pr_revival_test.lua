local conv_reconcile = require("devloop.convergence.reconcile")
local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local graph = require("testkit.graph")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local operator_commands = require("devloop.operator_commands")
local payloads_builders = require("devloop.payloads.builders")
local requests_review = require("devloop.requests.review")
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local t = h.t
local core = h.core

local repo = "owner/repo"
local issue_number = 42
local pr_number = 7
local proposal_id = "github-devloop/issue/owner/repo/42"
local pushed_head_sha = "feedface"

local function source_ref()
  return entity_lib.pr_source_ref(repo, pr_number)
end

local function entity_changed_event(updated_at)
  return {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "pr",
      repo = repo,
      number = pr_number,
      updated_at = updated_at,
      dedup_key = repo .. "#pr/" .. tostring(pr_number) .. "@" .. tostring(updated_at),
      source_ref = source_ref(),
    },
    source_ref = {
      kind = "external",
      reference = repo .. "#pr/" .. tostring(pr_number),
    },
  }
end

local function trusted_rereview_command()
  return {
    id = "IC_blocked_pr_rereview_command",
    body = "fkst: rereview",
    author_login = "fkst-test-bot",
    created_at = "2026-07-12T06:00:00Z",
  }
end

local function mock_env(times)
  for _ = 1, times or 8 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_BOT_LOGIN"), {
      stdout = "fkst-test-bot",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_UPSTREAM_BRANCH"), {
      stdout = "dev",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_INTEGRATION_BRANCH"), {
      stdout = "dev",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_pr_origin_read(comments, head_sha, labels)
  local fields = {
    repo = repo,
    number = pr_number,
    head = "devloop-owner-repo-42-01HY",
    head_sha = head_sha,
    base_branch = "dev",
    comments = comments,
    labels = labels or { "fkst-dev:blocked" },
    state = "OPEN",
    updated_at = "2026-07-12T05:15:00Z",
    times = 1,
  }
  entity_read_mocks.mock_pr_read_forms(t, fields)
  entity_read_mocks.mock_pr_view_selector(t, fields, entity_read_mocks.pr_origin_selector)
end

local function reconcile_drop_comments()
  local reconcile = h.review_reconcile()
  local blocked_version = conv_reconcile.review_reconcile_terminal_state_version(
    reconcile.issue_version,
    reconcile.round
  )
  local drop = core.build_review_reconcile_comment_request(
    repo,
    tostring(issue_number),
    reconcile,
    "drop",
    "no-semantic-progress-after-3-review-rounds",
    blocked_version
  ).body
  return {
    m_builders.pr_origin_marker(
      proposal_id,
      tostring(issue_number),
      "devloop-owner-repo-42-01HY",
      reconcile.issue_version,
      "dev"
    ),
    drop,
  }, blocked_version
end

return {
  test_reconcile_dropped_blocked_pr_head_push_is_inert = function()
    local blocked_comments = reconcile_drop_comments()

    mock_env()
    h.mock_default_issue_claim()
    mock_pr_origin_read(blocked_comments, pushed_head_sha)

    local head_push_trace = graph.require_quiescent(graph.run(
      entity_changed_event("2026-07-12T05:15:00Z"),
      { max_steps = 3 }
    ))
    graph.assert_covers(head_push_trace, {
      "github-proxy.github_entity_changed -> github-devloop-pr.observe_pr",
    })
    t.eq(graph.find_raise(head_push_trace, "github-proxy.github_pr_comment_request"), nil)
    t.eq(graph.find_raise(head_push_trace, "github-devloop-pr.devloop_reviewing"), nil)
    t.eq(graph.find_raise(head_push_trace, "consensus.proposal"), nil)
  end,

  test_reconcile_dropped_blocked_pr_rereview_uses_fresh_head_identity = function()
    local blocked_comments, blocked_version = reconcile_drop_comments()

    mock_env()
    local command = trusted_rereview_command()
    local command_comments = { table.unpack(blocked_comments) }
    table.insert(command_comments, command)
    local expected_version = operator_commands.operator_rereview_version(blocked_version, pushed_head_sha)
    local command_fact = operator_commands.operator_command_fact(command_comments, "rereview")
    local comment_request = requests_review.build_operator_rereview_comment_request(
      repo,
      pr_number,
      proposal_id,
      expected_version,
      command_fact,
      source_ref()
    )
    t.eq(comment_request.handoff.version, expected_version)
    t.is_true(comment_request.handoff.version ~= blocked_version)
    t.is_true(comment_request.body:find('state="reviewing"', 1, true) ~= nil)

    local reviewing = payloads_builders.build_devloop_reviewing_payload({
      proposal_id = proposal_id,
      impl_version = expected_version,
      reviewing_comment_id = "IC_blocked_pr_rereview",
    }, pr_number, source_ref(), expected_version)
    t.eq(reviewing.version, expected_version)

    local reviewing_comments = { table.unpack(command_comments) }
    table.insert(reviewing_comments, comment_request.body)
    mock_pr_origin_read(reviewing_comments, pushed_head_sha, { "fkst-dev:reviewing" })
    h.mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(proposal_id, "reviewing", expected_version),
    }, {
      repo = repo,
      number = issue_number,
      assignees = { "fkst-test-bot" },
      author_login = "fkst-test-bot",
    })
    local review = h.run_review_pr(reviewing, h.opts("blocked-pr-rereview-review"))
    local proposal = h.find_raise(review.raises, "consensus.proposal").payload
    t.eq(
      proposal.proposal_id,
      devloop_base.pr_review_proposal_id(repo, pr_number, expected_version, pushed_head_sha)
    )
    t.eq(proposal.source_ref.ref, repo .. "#pr/" .. tostring(pr_number))
  end,
}
