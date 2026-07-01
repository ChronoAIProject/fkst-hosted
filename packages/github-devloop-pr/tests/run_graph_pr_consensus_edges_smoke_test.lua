local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local convergence_shared = require("devloop.convergence.shared")
local transition_version = require("contract.transition_version")
local h = require("tests.devloop_helpers")
local graph = require("testkit.graph")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local conv_rounds = require("devloop.convergence.rounds")
local m_builders = require("devloop.markers.builders")

local t = h.t
local core = h.core

local repo = "owner/repo"
local issue_number = 42
local pr_number = 7
local issue_proposal_id = "github-devloop/issue/owner/repo/42"
local reviewed_version = transition_version.safe_version_segment(h.reviewing().version)
local reviewed_head_sha = "def456"
local review_proposal_id = devloop_base.pr_review_proposal_id(repo, pr_number, reviewed_version, reviewed_head_sha)
local review_dedup_key = "consensus:" .. review_proposal_id .. "/review"

local function pr_source_ref()
  return entity_lib.pr_source_ref(repo, pr_number)
end

local function initial_event(queue, payload)
  return {
    queue = queue,
    payload = payload,
    source_ref = {
      kind = "external",
      reference = repo .. "#pr/" .. tostring(pr_number),
    },
  }
end

local function state_marker(state)
  return core.state_marker(issue_proposal_id, state, reviewed_version)
end

local function pr_origin_marker()
  return m_builders.pr_origin_marker(core, issue_proposal_id, tostring(issue_number), "devloop-owner-repo-42-01HY", reviewed_version, "dev")
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
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_pr_origin_read(comments)
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = pr_number,
    head = "devloop-owner-repo-42-01HY",
    base_branch = "dev",
    head_sha = reviewed_head_sha,
    state = "OPEN",
    comments = comments,
  }, entity_read_mocks.pr_origin_selector)
end

local function seed_pr_and_issue_reads(state, extra_comments)
  local comments = { pr_origin_marker(), state_marker(state or "reviewing") }
  for _, comment in ipairs(extra_comments or {}) do
    table.insert(comments, comment)
  end
  mock_pr_origin_read(comments)
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = repo,
    number = issue_number,
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "assignees,author")
  h.mock_issue_review(
    { "fkst-dev:reviewing" },
    { state_marker(state or "reviewing") },
    { assignees = { "fkst-test-bot" }, author_login = "fkst-test-bot" }
  )
end

local function unresolved_angle_digests()
  return {
    { angle = "minimal", verdict = "comment", digest = "needs another pass" },
  }
end

local function narrowed_question()
  return "Is the PR ready to merge?"
end

local function unresolved_payload()
  return {
    schema = "consensus.consensus_converge.v1",
    proposal_id = review_proposal_id,
    dedup_key = review_dedup_key,
    source_ref = pr_source_ref(),
    round = 1,
    narrowed_question = narrowed_question(),
    angle_digests = unresolved_angle_digests(),
  }
end

local function reached_payload()
  return {
    schema = "consensus.consensus_reached.v1",
    proposal_id = review_proposal_id,
    decision = "approve",
    body = "Review consensus approves the diff.",
    dedup_key = review_dedup_key,
    source_ref = pr_source_ref(),
  }
end

local function review_converge_round_marker()
  return conv_rounds.review_converge_round_marker(core,
    review_proposal_id,
    issue_proposal_id,
    reviewed_version,
    reviewed_head_sha,
    convergence_shared.source_ref_digest(pr_source_ref()),
    1,
    review_dedup_key,
    narrowed_question(),
    unresolved_angle_digests()
  )
end

return {
  test_run_graph_pr_consensus_converge_routes_to_review_loop = function()
    mock_env()
    seed_pr_and_issue_reads("reviewing", { review_converge_round_marker() })

    local trace = graph.require_quiescent(graph.run(
      initial_event("consensus.consensus_converge", unresolved_payload()),
      { max_steps = 4 }
    ))
    graph.assert_covers(trace, {
      "consensus.consensus_converge -> github-devloop-pr.review_loop",
    })

    local step = graph.require_delivery(trace, {
      queue = "consensus.consensus_converge",
      consumer = "github-devloop-pr.review_loop",
    })
    t.eq(step.exit_code, 0)
  end,

  test_run_graph_pr_consensus_reached_routes_to_review_result = function()
    mock_env()
    seed_pr_and_issue_reads("merge-ready")
    t.mock_command("gh pr diff '7' --repo 'owner/repo' --name-only", {
      stdout = "file.lua\n",
      stderr = "",
      exit_code = 0,
    })

    local trace = graph.require_quiescent(graph.run(
      initial_event("consensus.consensus_reached", reached_payload()),
      { max_steps = 4 }
    ))
    graph.assert_covers(trace, {
      "consensus.consensus_reached -> github-devloop-pr.review_result",
    })

    local step = graph.require_delivery(trace, {
      queue = "consensus.consensus_reached",
      consumer = "github-devloop-pr.review_result",
    })
    t.eq(step.exit_code, 0)
  end,
}
