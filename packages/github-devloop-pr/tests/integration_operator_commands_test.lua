local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local requests_review = require("devloop.requests.review")
local convergence_shared = require("devloop.convergence.shared")
local operator_commands = require("devloop.operator_commands")
local transition_version = require("contract.transition_version")
local h = require("tests.devloop_helpers")
local payloads_builders = require("devloop.payloads.builders")
local conv_rounds = require("devloop.convergence.rounds")
local m_builders = require("devloop.markers.builders")
local t = h.t
local core = h.core
local opts = h.opts
local reviewing = h.reviewing
local merge_ready = h.merge_ready
local issue = h.issue
local reached = h.reached
local run_observe_pr = h.run_observe_pr
local run_observe = h.run_observe
local run_review_pr = h.run_review_pr
local mock_issue_reviewing = h.mock_issue_reviewing
local mock_issue_review = h.mock_issue_review
local mock_issue_state = h.mock_issue_state
local mock_pr_origin = h.mock_pr_origin
local merge_comments = h.merge_comments
local find_raise = h.find_raise
local find_causal_raise = h.find_causal_raise

local function pr_event(updated_at)
  return {
    schema = "github-proxy.v1",
    type = "pr",
    repo = "owner/repo",
    number = 7,
    dedup_key = "owner/repo#pr#7@" .. tostring(updated_at or "2026-06-04T03:00:00Z"),
    source_ref = {
      kind = "external",
      ref = "owner/repo#pr/7",
    },
  }
end

local function trusted_command(id)
  return {
    id = id or "IC_rereview_1",
    body = "fkst: rereview\n\nCI was rerun.",
    author_login = "fkst-test-bot",
    created_at = "2026-06-04T03:00:00Z",
  }
end

local function trusted_issue_command(command, id)
  return {
    id = id or ("IC_" .. tostring(command) .. "_issue_1"),
    body = "fkst: " .. tostring(command),
    author_login = "fkst-test-bot",
    created_at = "2026-06-04T03:00:00Z",
  }
end

local function thinking_converge_comments(event, rounds, command)
  local proposal_id = base_ids.proposal_id(event.repo, event.number)
  local base_version = payloads_builders.build_proposal(core, event).dedup_key
  local sr_digest = convergence_shared.source_ref_digest(event.source_ref)
  local angle_digests = {
    { angle = "minimal", verdict = "abstain", digest = "same-digest" },
  }
  local comments = {
    core.state_marker(proposal_id, "thinking", base_version .. "/loop/" .. tostring(rounds)),
  }
  for n = 1, rounds do
    table.insert(comments, conv_rounds.converge_round_marker(core,
      proposal_id,
      base_version,
      sr_digest,
      n,
      base_version .. "/loop/" .. tostring(n),
      "Same narrowed question",
      angle_digests
    ))
  end
  if command ~= nil then
    table.insert(comments, command)
  end
  return comments, base_version
end

local function thinking_changing_converge_comments(event, rounds, command)
  local proposal_id = base_ids.proposal_id(event.repo, event.number)
  local base_version = payloads_builders.build_proposal(core, event).dedup_key
  local sr_digest = convergence_shared.source_ref_digest(event.source_ref)
  local comments = {
    core.state_marker(proposal_id, "thinking", base_version .. "/loop/" .. tostring(rounds)),
  }
  for n = 1, rounds do
    table.insert(comments, conv_rounds.converge_round_marker(core,
      proposal_id,
      base_version,
      sr_digest,
      n,
      base_version .. "/loop/" .. tostring(n),
      "Narrowed question " .. tostring(n),
      {
        { angle = "minimal", verdict = "abstain", digest = "digest-" .. tostring(n) },
      }
    ))
  end
  if command ~= nil then
    table.insert(comments, command)
  end
  return comments, base_version
end

local function find_issue_comment_raise(raises, needle)
  for _, raised in ipairs(raises or {}) do
    if raised.queue == "github-proxy.github_issue_comment_request"
      and raised.payload.body:find(needle, 1, true) ~= nil then
      return raised
    end
  end
  return nil
end

return {
  test_trusted_rereview_command_reenters_reviewing = function()
    local impl_version = reviewing().version
    local command = trusted_command()
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "blocked", impl_version .. "/review-loop/3"),
      command,
    }, "devloop-owner-repo-42-01HY", "feedface")

    local result = run_observe_pr(pr_event(), opts("operator-rereview"))
    t.eq(result.exit_code, 0)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local reviewing_raise = find_causal_raise(result, "devloop_reviewing")
    t.is_true(comment_raise.payload.body:find("operator command accepted: rereview", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find("fkst:github-devloop:operator-command:v1", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('state="reviewing"', 1, true) ~= nil)
    t.eq(reviewing_raise.payload.version, impl_version .. "/review-loop/3/review-loop/4/rereview/4/feedface")
    t.eq(reviewing_raise.payload.source_ref.ref, "owner/repo#pr/7")

    mock_issue_review({ "fkst-dev:reviewing" }, {
      comment_raise.payload.body,
    })
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", reviewing_raise.payload.version),
      comment_raise.payload.body,
    }, "devloop-owner-repo-42-01HY", "feedface")
    local review = run_review_pr(reviewing_raise.payload, opts("operator-rereview-review"))
    t.eq(review.exit_code, 0)
    t.eq(find_raise(review.raises, "consensus.proposal"), nil)
  end,

  test_untrusted_rereview_command_is_ignored = function()
    local impl_version = reviewing().version
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "blocked", impl_version .. "/review-loop/3"),
      {
        id = "IC_rereview_untrusted",
        body = "fkst: rereview",
        author_login = "ordinary-user",
        created_at = "2026-06-04T03:00:00Z",
      },
    })

    local result = run_observe_pr(pr_event(), opts("operator-rereview-untrusted"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request"), nil)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
  end,

  test_rereview_command_invalid_state_refuses_once = function()
    local impl_version = reviewing().version
    local command = trusted_command("IC_rereview_invalid")
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "merge-ready", impl_version),
      command,
    })
    mock_issue_reviewing({ "fkst-dev:merge-ready" }, merge_comments(merge_ready()))

    local result = run_observe_pr(pr_event(), opts("operator-rereview-invalid"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment_raise.payload.body:find("operator command refused", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('outcome="refused"', 1, true) ~= nil)

    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "merge-ready", impl_version),
      command,
      comment_raise.payload.body,
    })
    mock_issue_reviewing({ "fkst-dev:merge-ready" }, merge_comments(merge_ready()))
    local replay = run_observe_pr(pr_event("2026-06-04T03:01:00Z"), opts("operator-rereview-invalid-replay"))
    t.eq(replay.exit_code, 0)
    t.is_true(find_raise(replay.raises, "github-proxy.github_pr_comment_request") ~= nil)
  end,

  test_rereview_command_active_reviewing_refuses = function()
    local impl_version = reviewing().version
    local command = trusted_command("IC_rereview_active_reviewing")
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
      command,
    }, "devloop-owner-repo-42-01HY", "feedface")

    local result = run_observe_pr(pr_event(), opts("operator-rereview-active-reviewing"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment_raise.payload.body:find("operator command refused", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('outcome="refused"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find("stalled reviewing state", 1, true) ~= nil)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
  end,

  test_rereview_command_stalled_reviewing_reenters_reviewing = function()
    local impl_version = reviewing().version
    local command = trusted_command("IC_rereview_stalled_reviewing")
    local review_proposal = devloop_base.pr_review_proposal_id("owner/repo", 7, impl_version, "feedface")
    local review_version = transition_version.safe_version_segment(impl_version)
    local sr_digest = convergence_shared.source_ref_digest({ kind = "external", ref = "owner/repo#pr/7" })
    local angle_digests = {
      { angle = "minimal", verdict = "abstain", digest = "same-review-digest" },
    }
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
      conv_rounds.review_converge_round_marker(core, review_proposal, "github-devloop/issue/owner/repo/42", review_version, "feedface", sr_digest, 1, "base", "Same review question", angle_digests),
      conv_rounds.review_converge_round_marker(core, review_proposal, "github-devloop/issue/owner/repo/42", review_version, "feedface", sr_digest, 2, "loop1", "Same review question", angle_digests),
      conv_rounds.review_converge_round_marker(core, review_proposal, "github-devloop/issue/owner/repo/42", review_version, "feedface", sr_digest, 3, "loop2", "Same review question", angle_digests),
      command,
    }, "devloop-owner-repo-42-01HY", "feedface")

    local result = run_observe_pr(pr_event(), opts("operator-rereview-stalled-reviewing"))
    t.eq(result.exit_code, 0)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local reviewing_raise = find_causal_raise(result, "devloop_reviewing")
    t.is_true(comment_raise.payload.body:find("operator command accepted: rereview", 1, true) ~= nil)
    t.eq(reviewing_raise.payload.version, impl_version .. "/review-loop/1/rereview/1/feedface")
  end,

  test_rereview_command_duplicate_response_is_idempotent = function()
    local impl_version = reviewing().version
    local command = trusted_command("IC_rereview_duplicate")
    local command_fact = operator_commands.operator_command_fact(core, { command }, "rereview")
    local response = requests_review.build_operator_rereview_comment_request(core,
      "owner/repo",
      7,
      "github-devloop/issue/owner/repo/42",
      impl_version .. "/review-loop/4/rereview/4/feedface",
      command_fact,
      { kind = "external", ref = "owner/repo#pr/7" }
    ).body
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "blocked", impl_version .. "/review-loop/3"),
      command,
      response,
    }, "devloop-owner-repo-42-01HY", "feedface")

    local result = run_observe_pr(pr_event(), opts("operator-rereview-duplicate"))
    t.eq(result.exit_code, 0)
    local reviewing_raise = find_causal_raise(result, "devloop_reviewing")
    t.eq(reviewing_raise.payload.version, impl_version .. "/review-loop/4/rereview/4/feedface")
  end,
}
