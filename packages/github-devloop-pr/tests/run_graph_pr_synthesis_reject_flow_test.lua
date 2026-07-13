local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local transition_version = require("contract.transition_version")
local h = require("tests.devloop_helpers")
local graph = require("testkit.graph")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")

local t = h.t
local core = h.core

local verdict_label = "⟦FKST:VERDICT⟧"
local reply_label = "⟦FKST:REPLY⟧"
local gap_label = "⟦FKST:GAP⟧"
local stance_label = "⟦FKST:STANCE⟧"
local gap = "missing regression test"
local repo = "owner/repo"
local issue_number = 42
local pr_number = 7
local issue_proposal_id = "github-devloop/issue/owner/repo/42"
local reviewed_version = transition_version.safe_version_segment(h.reviewing().version)
local reviewed_head_sha = "def456"
local review_proposal_id = devloop_base.pr_review_proposal_id(repo, pr_number, reviewed_version, reviewed_head_sha)
local review_dedup_key = review_proposal_id .. "/review"

local function pr_source_ref()
  return entity_lib.pr_source_ref(repo, pr_number)
end

local function proposal()
  return {
    schema = "consensus.proposal.v1",
    proposal_id = review_proposal_id,
    title = "Review PR diff",
    body = "Decide whether the reviewed PR diff is safe to merge.",
    context = "Reject only for a goal-blocking review gap.",
    angles = { "teleology", "parsimony", "fidelity" },
    verdict_mode = "gate",
    dedup_key = review_dedup_key,
    source_ref = pr_source_ref(),
  }
end

local function initial_event()
  return {
    queue = "consensus.proposal",
    payload = proposal(),
    source_ref = {
      kind = "external",
      reference = repo .. "#pr/" .. tostring(pr_number),
    },
  }
end

local function mock_result(command, stdout)
  t.mock_command(command, {
    stdout = stdout,
    stderr = "",
    exit_code = 0,
  })
end

local function mock_consensus()
  mock_result('printf %s "$FKST_RUNTIME_ROOT"', "/tmp/fkst-packages-test/github-devloop-pr-synthesis-reject/runtime")
  for _ = 1, 7 do
    mock_result("mkdir -p", "")
  end

  mock_result("consensus-angle-teleology", table.concat({
    verdict_label .. " comment",
    reply_label .. " The implementation shape is acceptable but needs verification.",
  }, "\n") .. "\n")
  mock_result("consensus-angle-parsimony", table.concat({
    verdict_label .. " abstain",
    reply_label .. " The current evidence does not settle readiness.",
  }, "\n") .. "\n")
  mock_result("consensus-angle-fidelity", table.concat({
    verdict_label .. " comment",
    reply_label .. " Preserve the existing review ownership contract.",
  }, "\n") .. "\n")

  mock_result("consensus-rebuttal-teleology", table.concat({
    stance_label .. " defend",
    verdict_label .. " reject",
    reply_label .. " The diff changes behavior without a regression test.",
    gap_label .. " " .. gap,
  }, "\n") .. "\n")
  mock_result("consensus-rebuttal-parsimony", table.concat({
    stance_label .. " defend",
    verdict_label .. " comment",
    reply_label .. " Keep the repair limited to the missing test.",
  }, "\n") .. "\n")
  mock_result("consensus-rebuttal-fidelity", table.concat({
    stance_label .. " defend",
    verdict_label .. " abstain",
    reply_label .. " No additional blocking gap is established.",
  }, "\n") .. "\n")

  mock_result("consensus-synthesis-", table.concat({
    "reached:reject reject until the Phase R gap is fixed",
    gap_label .. " " .. gap,
  }, "\n") .. "\n")
end

local function state_marker()
  return core.state_marker(issue_proposal_id, "reviewing", reviewed_version)
end

local function mock_review_result_reads()
  for _ = 1, 8 do
    mock_result(devloop_base.read_env_command("FKST_GITHUB_WRITE"), "")
  end
  for _ = 1, 8 do
    mock_result(devloop_base.read_env_command("FKST_GITHUB_BOT_LOGIN"), "fkst-test-bot")
    mock_result(devloop_base.read_env_command("FKST_DEVLOOP_UPSTREAM_BRANCH"), "dev")
    mock_result(devloop_base.read_env_command("FKST_DEVLOOP_INTEGRATION_BRANCH"), "")
  end

  entity_read_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = pr_number,
    head = "devloop-owner-repo-42-01HY",
    base_branch = "dev",
    head_sha = reviewed_head_sha,
    state = "OPEN",
    comments = {
      m_builders.pr_origin_marker(issue_proposal_id, tostring(issue_number), "devloop-owner-repo-42-01HY", reviewed_version, "dev"),
      state_marker(),
    },
  }, entity_read_mocks.pr_origin_selector)
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = repo,
    number = issue_number,
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "assignees,author")
  h.mock_issue_review({ "fkst-dev:reviewing" }, { state_marker() }, {
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  })
  mock_result("gh pr diff '7' --repo 'owner/repo' --name-only", "file.lua\n")
end

local function has_value(values, expected)
  for _, value in ipairs(values or {}) do
    if value == expected then
      return true
    end
  end
  return false
end

return {
  test_run_graph_gate_synthesis_reject_advances_pr_review_to_fixing = function()
    mock_consensus()
    mock_review_result_reads()

    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 4 }))
    graph.assert_covers(trace, {
      "consensus.proposal -> consensus.decide",
      "consensus.consensus_reached -> github-devloop-pr.review_result",
    })

    local decide_step = graph.require_delivery(trace, {
      queue = "consensus.proposal",
      consumer = "consensus.decide",
    })
    local review_step = graph.require_delivery(trace, {
      queue = "consensus.consensus_reached",
      consumer = "github-devloop-pr.review_result",
    })
    t.eq(decide_step.exit_code, 0)
    t.eq(review_step.exit_code, 0)

    local reached = graph.require_raise(trace, "consensus.consensus_reached")
    t.eq(reached.payload.decision, "reject")
    t.eq(reached.payload.blocking_gap, gap)
    t.eq(reached.payload.blocking_gaps[1], gap)

    local comment = graph.require_raise(trace, "github-proxy.github_pr_comment_request")
    t.is_true(comment.payload.body:find('decision="reject"', 1, true) ~= nil)
    t.is_true(comment.payload.body:find('state="fixing"', 1, true) ~= nil)
    t.eq(comment.payload.handoff.kind, "github-devloop.fixing")
    t.eq(comment.payload.handoff.blocking_gap, gap)

    local label = graph.require_raise(trace, "github-proxy.github_issue_label_request")
    t.is_true(has_value(label.payload.add_labels, "fkst-dev:fixing"))
    t.is_nil(graph.find_raise(trace, "github-devloop-pr.devloop_merge_ready"))
    t.is_nil(graph.find_raise(trace, "devloop_merge_ready"))
    t.is_nil(graph.find_raise(trace, "github-proxy.github_pr_comment_request", function(raised)
      return raised.payload.handoff ~= nil and raised.payload.handoff.kind == "github-devloop.merge_ready"
    end))
  end,
}
