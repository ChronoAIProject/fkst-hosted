local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local requests_review = require("devloop.requests.review")
local h = require("tests.devloop_helpers")
local graph = require("testkit.graph")

local t = h.t
local core = h.core

local repo = "owner/repo"
local pr_number = 7
local comment_id = "123456"
local head_sha = "def456"
local version = "pr-native-version/review/1"
local proposal_id = entity_lib.pr_proposal_id(repo, pr_number)
local review_proposal_id = devloop_base.pr_review_proposal_id(repo, pr_number, version, head_sha)
local review_dedup_key = "consensus:" .. review_proposal_id .. "/review"

local function source_ref()
  return entity_lib.pr_source_ref(repo, pr_number)
end

local function review_result_comment_request()
  return requests_review.build_review_result_comment_request(core, repo, nil, proposal_id, version, {
    schema = "consensus.consensus_reached.v1",
    proposal_id = review_proposal_id,
    decision = "approve",
    body = "Review consensus approves the PR-native diff.",
    dedup_key = review_dedup_key,
    source_ref = source_ref(),
    current_head_sha = head_sha,
    angle_results = {
      { angle = "minimal", verdict = "approve" },
      { angle = "structural", verdict = "approve" },
      { angle = "delete", verdict = "approve" },
    },
  }, source_ref())
end

local function initial_event()
  return {
    queue = "github-proxy.github_pr_comment_request",
    payload = review_result_comment_request(),
    source_ref = {
      kind = "external",
      reference = repo .. "#pr/" .. tostring(pr_number),
    },
  }
end

local function mock_runtime_and_context()
  for _ = 1, 8 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_WRITE"), {
      stdout = "1",
      stderr = "",
      exit_code = 0,
    })
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

local function mock_pr_comment_write()
  for _, command in ipairs({
    "gh api --paginate --slurp repos/owner/repo/issues/7/comments?per_page=100",
    "gh api --paginate --slurp 'repos/owner/repo/issues/7/comments?per_page=100'",
  }) do
    t.mock_command(command, {
      stdout = "[[]]\n",
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command("gh api --method POST repos/owner/repo/issues/7/comments --field 'body=", {
    stdout = '{"id":' .. comment_id .. ',"body":"created","user":{"login":"fkst-test-bot"}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_handoff_marker_visibility()
  t.mock_command("gh api --method GET 'repos/owner/repo/issues/comments/" .. comment_id .. "'", {
    stdout = '{"body":"' .. h.json_string(core.state_marker(proposal_id, "merge-ready", version)) .. '","user":{"login":"fkst-test-bot"}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_label_write()
  for _, command in ipairs({
    "gh api --paginate --slurp repos/owner/repo/issues/7/comments?per_page=100",
    "gh api --paginate --slurp 'repos/owner/repo/issues/7/comments?per_page=100'",
  }) do
    t.mock_command(command, {
      stdout = '[[{"id":' .. comment_id .. ',"body":"' .. h.json_string(core.state_marker(proposal_id, "merge-ready", version)) .. '","user":{"login":"fkst-test-bot"}}]]\n',
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command("gh label list", {
    stdout = '[{"name":"fkst-dev:thinking"},{"name":"fkst-dev:ready"},{"name":"fkst-dev:implementing"},{"name":"fkst-dev:awaiting-pr"},{"name":"fkst-dev:pr-open"},{"name":"fkst-dev:reviewing"},{"name":"fkst-dev:merge-ready"},{"name":"fkst-dev:merging"},{"name":"fkst-dev:merged"},{"name":"fkst-dev:blocked"},{"name":"fkst-dev:fixing"},{"name":"fkst-dev:review-meta"},{"name":"fkst-dev:impl-failed"}]\n',
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh pr edit", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_run_graph_pr_review_comment_handoffs_to_merge_ready = function()
    mock_runtime_and_context()
    mock_pr_comment_write()
    mock_handoff_marker_visibility()
    mock_pr_label_write()

    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 6 }))
    graph.assert_covers(trace, {
      "github-proxy.github_pr_comment_request -> github-proxy.github_pr_comment",
      "github-proxy.github_comment_written -> github-devloop-pr.comment_handoff",
      "github-proxy.github_issue_label_request -> github-proxy.github_issue_label",
    })

    local comment_step, comment_index = graph.require_delivery(trace, {
      queue = "github-proxy.github_pr_comment_request",
      consumer = "github-proxy.github_pr_comment",
    })
    t.eq(comment_step.exit_code, 0)

    local written, _, written_index = graph.require_raise(
      trace,
      "github-proxy.github_comment_written",
      function(raised)
        return raised.payload.handoff ~= nil
          and raised.payload.handoff.kind == "github-devloop.merge_ready"
          and tostring(raised.payload.comment_id) == comment_id
      end
    )
    t.eq(written_index, comment_index)
    t.eq(written.payload.target, "pr")
    t.eq(tostring(written.payload.pr_number), tostring(pr_number))
    t.is_true(written.payload.dedup_key:find("/written/" .. comment_id, 1, true) ~= nil)

    local handoff_step, handoff_index = graph.require_delivery(trace, {
      queue = "github-proxy.github_comment_written",
      consumer = "github-devloop-pr.comment_handoff",
    })
    t.eq(handoff_step.exit_code, 0)
    t.is_true(handoff_index > written_index)

    local merge_ready, _, merge_ready_index = graph.require_raise(
      trace,
      "github-devloop-pr.devloop_merge_ready",
      function(raised)
        return raised.payload.proposal_id == proposal_id
      end
    )
    t.eq(merge_ready_index, handoff_index)
    t.eq(merge_ready.payload.schema, "github-devloop.merge-ready.v1")
    t.eq(tostring(merge_ready.payload.pr_number), tostring(pr_number))
    t.eq(merge_ready.payload.version, version)
    t.eq(merge_ready.payload.review_proposal_id, review_proposal_id)
    t.eq(merge_ready.payload.review_dedup_key, review_dedup_key)
    t.eq(merge_ready.payload.reviewed_head_sha, head_sha)

    local label_step, label_index = graph.require_delivery(trace, {
      queue = "github-proxy.github_issue_label_request",
      consumer = "github-proxy.github_issue_label",
    })
    t.eq(label_step.exit_code, 0)
    t.is_true(label_index > merge_ready_index)

    local merge_step, merge_index = graph.require_delivery(trace, {
      queue = "github-devloop-pr.devloop_merge_ready",
      consumer = "github-devloop-pr.merge",
    })
    t.eq(merge_step.exit_code, 0)
    t.is_true(merge_index > merge_ready_index)
  end,
}
