local devloop_base = require("devloop.base")
local h = require("tests.devloop_helpers")
local payloads_builders = require("devloop.payloads.builders")
local m_facts = require("devloop.markers.facts")
local m_builders = require("devloop.markers.builders")
local t = h.t
local core = h.core
local action_label = h.action_label
local reason_label = h.reason_label
local opts = h.opts
local reviewing = h.reviewing
local review_reached = h.review_reached
local review_meta_event = h.review_meta_event
local run_review_result = h.run_review_result
local run_review_meta = h.run_review_meta
local mock_issue_result = h.mock_issue_result
local mock_issue_review_meta = h.mock_issue_review_meta
local mock_pr_origin = h.mock_pr_origin
local find_raise = h.find_raise
local find_causal_raise = h.find_causal_raise

local function reflection_review_version()
  return core.next_fix_version(core.next_fix_version(reviewing().version))
end

local function reflection_meta_event()
  return review_meta_event({
    mode = "fix-reflection",
    fix_round = 3,
    version = core.fix_version_from_review_version(core.next_fix_version(reviewing().version)),
    blocking_gap = "missing regression guard",
  })
end

local function mock_reflection_context(event, ledger)
  mock_issue_review_meta({ "fkst-dev:review-meta" }, {
    core.state_marker(event.proposal_id, "review-meta", event.version),
    m_builders.fix_reflection_marker(core, event.proposal_id, event.dedup_key, "checkpoint", event.version, 3),
    ledger,
  })
  h.mock_context_bundle()
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
end

return {
  test_review_result_third_fix_round_enters_reflection_checkpoint = function()
    local review_version = reflection_review_version()
    local review_proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, review_version, "def456")
    local event = review_reached({
      proposal_id = review_proposal_id,
      dedup_key = "consensus:" .. review_proposal_id .. "/review",
      decision = "reject",
      body = "Review consensus rejects the diff.",
      blocking_gap = "missing regression guard",
    })
    local reflection_version = core.fix_version_from_review_version(review_version)
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", review_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", review_version),
    })

    local result = run_review_result(event, opts("review-result-fix-reflection"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    local reflection_raise = find_raise(result.raises, "devloop_review_meta")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:review-meta")
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    t.is_true(comment_raise.payload.body:find("fkst:github-devloop:fix-reflection:v1", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('state="review-meta" version="' .. reflection_version .. '"', 1, true) ~= nil)
    t.eq(reflection_raise.payload.mode, "fix-reflection")
    t.eq(reflection_raise.payload.fix_round, 3)
    t.eq(reflection_raise.payload.version, reflection_version)
    t.eq(reflection_raise.payload.blocking_gap, "missing regression guard")
  end,

  test_fix_reflection_continue_resumes_fixing_with_review_meta_fact = function()
    local event = reflection_meta_event()
    mock_reflection_context(event, "Round ledger: gaps stayed aligned with the issue goal.")
    t.mock_command("codex exec", {
      stdout = action_label .. " continue\n" .. reason_label .. " The fix rounds are still converging on the original goal.",
      stderr = "",
      exit_code = 0,
    })

    local result = run_review_meta(event, opts("fix-reflection-continue"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment = find_raise(result.raises, "github-proxy.github_pr_comment_request").payload.body
    local fix_raise = find_causal_raise(result, "devloop_fixing")
    local exit_version = core.next_review_meta_action_version(event.version)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:fixing")
    t.eq(fix_raise.payload.blocking_gap, "missing regression guard")
    t.is_true(comment:find("fkst:github-devloop:fix-reflection:v1", 1, true) ~= nil)
    t.is_true(comment:find('verdict="continue"', 1, true) ~= nil)
    t.is_true(comment:find("fkst:github-devloop:review-meta:v1", 1, true) ~= nil)
    t.eq(m_facts.review_meta_fix_fact(core, { comment }, event.proposal_id, exit_version).blocking_gap, "missing regression guard")
  end,

  test_fix_reflection_replay_fact_restores_blocking_gap = function()
    local issue_version = core.fix_version_from_review_version(reflection_review_version())
    local review_version = core._strip_latest_fix_version_suffix(issue_version)
    local review_proposal = devloop_base.pr_review_proposal_id("owner/repo", 7, review_version, "def456")
    local review_dedup = "consensus:" .. review_proposal .. "/review"
    local fresh_payload = payloads_builders.build_devloop_fix_reflection_payload(core, {
      proposal_id = review_proposal,
      dedup_key = review_dedup,
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    }, "github-devloop/issue/owner/repo/42", issue_version, 7, 3, { kind = "external", ref = "owner/repo#pr/7" })
    fresh_payload.blocking_gap = "missing regression guard"
    local comments = {
      {
        author_login = core._test_bot_login,
        body = table.concat({
          core.state_marker("github-devloop/issue/owner/repo/42", "review-meta", issue_version),
          m_builders.review_result_marker(core, review_proposal, "github-devloop/issue/owner/repo/42", "reject", review_dedup, 3, "missing regression guard"),
          m_builders.fix_reflection_marker(core, "github-devloop/issue/owner/repo/42", review_dedup, "checkpoint", issue_version, 3),
        }, "\n"),
        created_at = "2026-06-03T01:02:03Z",
      },
    }

    local fact = core.review_meta_replay_fact_from_state(comments, "github-devloop/issue/owner/repo/42", issue_version, 7, "def456", 0)
    t.eq(fact.mode, "fix-reflection")
    t.eq(fact.fix_round, 3)
    t.eq(fact.blocking_gap, "missing regression guard")
    t.eq(fact.review_dedup_key, review_dedup)
    t.eq(fact.dedup_key, fresh_payload.dedup_key)

    local replay_payload = payloads_builders.build_devloop_fix_reflection_payload(core, fact, "github-devloop/issue/owner/repo/42", issue_version, fact.pr_number, fact.fix_round, fact.source_ref)
    replay_payload.blocking_gap = fact.blocking_gap
    t.eq(replay_payload.dedup_key, fresh_payload.dedup_key)
    t.eq(replay_payload.review_dedup_key, fresh_payload.review_dedup_key)
    t.eq(replay_payload.mode, fresh_payload.mode)
    t.eq(replay_payload.fix_round, fresh_payload.fix_round)
    t.eq(replay_payload.blocking_gap, fresh_payload.blocking_gap)
  end,

  test_fix_reflection_spec_gap_blocks_without_spawning_intake_issue = function()
    local event = reflection_meta_event()
    mock_reflection_context(event, "Round ledger: latest gap diverges from stated acceptance.")
    t.mock_command("codex exec", {
      stdout = action_label .. " spec-gap\n" .. reason_label .. " The review demand exceeds the original acceptance boundary.",
      stderr = "",
      exit_code = 0,
    })

    local result = run_review_meta(event, opts("fix-reflection-spec-gap"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment = find_raise(result.raises, "github-proxy.github_pr_comment_request").payload.body
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:blocked")
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    t.is_true(comment:find('verdict="spec-gap"', 1, true) ~= nil)
    t.is_true(comment:find('state="blocked"', 1, true) ~= nil)
    t.is_true(comment:find("The review demand exceeds the original acceptance boundary.", 1, true) ~= nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_create_request"), nil)
  end,
}
