local devloop_base = require("devloop.base")
local graph = require("testkit.graph")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local replay_fields = require("devloop.replay_fields")
local payloads_builders = require("devloop.payloads.builders")

local t = h.t
local core = h.core

local function polluted_merge_gate_comments(event)
  local clean_body = "github-devloop merge gate failed: mergeable-conflicting\n"
    .. m_builders.merge_gate_marker(event.proposal_id,
      event.pr_number,
      event.version,
      event.review_proposal_id,
      event.review_dedup_key,
      event.reviewed_head_sha,
      event.gate_baseline_sha,
      "mergeable-conflicting"
    )
  local malformed_body = "github-devloop merge gate failed: mergeable-conflicting\n"
    .. '<!-- fkst:github-devloop:merge-gate:v1 proposal="' .. event.proposal_id
    .. '" pr="' .. tostring(event.pr_number)
    .. '" version="' .. event.version
    .. '" review_proposal="' .. event.review_proposal_id
    .. '" review_dedup="' .. event.review_dedup_key
    .. '" head_sha="' .. event.reviewed_head_sha
    .. '" gate_baseline_sha="828df8d3" reason="' .. clean_body .. '" -->'
  return {
    {
      body = clean_body,
      author_login = "fkst-test-bot",
      created_at = "2026-07-11T21:14:00Z",
    },
    {
      body = malformed_body,
      author_login = "fkst-test-bot",
      created_at = "2026-07-11T21:19:00Z",
    },
  }
end

local function mock_fix_execution(event, comments)
  local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
  local origin_marker = m_builders.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
  local issue_comments = {
    core.state_marker(event.proposal_id, "fixing", event.version),
    comments[1],
    comments[2],
  }

  h.mock_bot_env()
  h.mock_write_env("1")
  h.mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, issue_comments, branch, event.version)
  h.mock_pr_fix({ origin_marker }, branch, event.reviewed_head_sha)
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
    stderr = "",
    exit_code = 0,
  })
  h.mock_existing_fix_worktree(branch, event.reviewed_head_sha, nil, {
    sha = event.gate_baseline_sha,
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  h.mock_implement_codex(0, "fixed polluted merge-gate replay")
  h.mock_git_status(" M packages/github-devloop-pr/core.lua\n")
  h.mock_git_commit("feedface", branch)
  h.mock_write_env("1")
  h.mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, issue_comments, branch, event.version)
  h.mock_git_push(branch)
  h.mock_pr_fix({ origin_marker }, branch, "feedface")
end

local function replay_fixing_comment_request(event, comments)
  local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
  local state = { state = "fixing", version = event.version, proposal_id = event.proposal_id }
  local current_pr = {
    number = event.pr_number,
    comments = comments,
    head_ref_name = branch,
    head_sha = event.reviewed_head_sha,
    base_ref_name = "dev",
    state = "OPEN",
  }
  local emitted = {}
  local tools = {
    find_linked_pr = function(snapshot, number)
      for _, item in ipairs(snapshot and snapshot.prs or {}) do
        if tostring(item.number or "") == tostring(number or "") then return item.current end
      end
      return nil
    end,
    log_skip = function(_, _, _, _, _, outcome, reason)
      error("unexpected fixing replay decline: " .. tostring(outcome) .. ": " .. tostring(reason))
    end,
    raise_effects = function(_, _, _, _, _, effects)
      for _, effect in ipairs(effects or {}) do table.insert(emitted, effect) end
      return true
    end,
  }
  local replayed = core.replayer_review_registry(tools).fixing(
    "observe_pr",
    { repo = "owner/repo", number = 42, source_ref = h.source_ref() },
    state,
    replay_fields.restart_transition_row(core.restart_transition_table(), "fixing"),
    {
      proposal_id = event.proposal_id,
      state = state,
      source_ref = event.source_ref,
      link = {
        proposal_id = event.proposal_id,
        pr_number = event.pr_number,
        branch = branch,
        impl_version = event.version,
        base_branch = "dev",
      },
      snapshot = {
        comments = comments,
        state = state,
        prs = { { number = event.pr_number, current = current_pr } },
      },
    }
  )
  t.eq(replayed, true)
  return h.find_raise(emitted, "github-proxy.github_pr_comment_request")
end

local function fixing_event_from_replay_request(request)
  local handoff = request.payload.handoff
  local fixing = payloads_builders.build_devloop_fixing_payload({
    proposal_id = handoff.proposal_id,
    impl_version = handoff.version,
  }, handoff.pr_number, handoff, handoff.source_ref)
  fixing.dedup_key = handoff.dedup_key or fixing.dedup_key
  return fixing
end

return {
  test_run_graph_polluted_merge_gate_stream_replays_clean_lineage = function()
    local event = h.fixing({
      gate_baseline_sha = "281c4f9e",
      gate_failure_excerpt = "mergeable-conflicting",
    })
    local comments = polluted_merge_gate_comments(event)
    local replay_request = replay_fixing_comment_request(event, comments)
    t.eq(replay_request.payload.handoff.gate_baseline_sha, event.gate_baseline_sha)
    h.mock_write_env("")

    local trace = graph.run({
      queue = "github-proxy.github_pr_comment_request",
      payload = replay_request.payload,
      source_ref = {
        kind = "external",
        reference = "owner/repo#pr/7",
      },
    }, { max_steps = 1 })
    graph.assert_covers(trace, {
      "github-proxy.github_pr_comment_request -> github-proxy.github_pr_comment",
    })
    graph.require_quiescent(trace)
    t.eq(trace.final.dead_letters, 0)
  end,

  test_polluted_merge_gate_clean_lineage_fix_executes_once = function()
    local expected = h.fixing({
      gate_baseline_sha = "281c4f9e",
      gate_failure_excerpt = "mergeable-conflicting",
    })
    local comments = polluted_merge_gate_comments(expected)
    local event = fixing_event_from_replay_request(replay_fixing_comment_request(expected, comments))
    t.eq(event.proposal_id, expected.proposal_id)
    t.eq(event.pr_number, expected.pr_number)
    t.eq(event.version, expected.version)
    t.eq(event.review_proposal_id, expected.review_proposal_id)
    t.eq(event.review_dedup_key, expected.review_dedup_key)
    t.eq(event.reviewed_head_sha, expected.reviewed_head_sha)
    t.eq(event.gate_baseline_sha, expected.gate_baseline_sha)
    t.eq(event.predecessor_set, expected.predecessor_set)
    t.eq(event.ci_failure_key, expected.ci_failure_key)
    t.eq(event.source_ref.ref, expected.source_ref.ref)
    mock_fix_execution(event, comments)

    local result = h.run_fix(event, h.opts("fix-polluted-merge-gate-clean-lineage", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("codex exec"), 1)
    local reviewing_request = h.find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.eq(reviewing_request.payload.handoff.kind, "github-devloop.reviewing")
  end,
}
