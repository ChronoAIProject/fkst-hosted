local devloop_base = require("devloop.base")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local find_raise = h.find_raise
local find_causal_raise = h.find_causal_raise
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")

local repo = "owner/repo"
local proposal_id = "github-devloop/issue/owner/repo/42"
local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local branch = "devloop-owner-repo-42-01HY"

local function pr_event()
  return {
    schema = "github-proxy.v1",
    type = "pr",
    repo = repo,
    number = 7,
    state = "OPEN",
    updated_at = "2026-06-04T01:02:03Z",
    labels = {},
    dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
    source_ref = h.pr_source_ref(),
  }
end

local function mock_self_owned_issue()
  t.mock_command(core.gh_issue_view_claim_cmd(repo, 42), {
    stdout = '{"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(core.gh_issue_view_result_cmd(repo, 42), {
    stdout = '{"labels":[{"name":"fkst-dev:fixing"}],"comments":[]}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr(state_name, mergeable, merge_state, extra_comments)
  local comments = {
    m_builders.pr_origin_marker(core, proposal_id, "42", branch, version, "dev"),
    core.state_marker(proposal_id, state_name, version),
  }
  for _, comment in ipairs(extra_comments or {}) do
    table.insert(comments, comment)
  end
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = 7,
    comments = comments,
    head = branch,
    head_sha = "def456",
    base_branch = "dev",
    state = "OPEN",
    mergeable = mergeable or "CONFLICTING",
    merge_state = merge_state or "DIRTY",
  }, entity_read_mocks.pr_origin_selector)
end

local function run_observe_pr_mergeability(name)
  h.mock_bot_env()
  mock_self_owned_issue()
  return t.run_department("departments/observe_pr/main.lua", {
    queue = "github-proxy.github_entity_changed",
    payload = pr_event(),
  }, opts(name))
end

local function assert_conflict_redrive(result, expected_from_state)
  t.eq(result.exit_code, 0)
  local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
  local issue_label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request", function(payload)
    return tostring(payload.target_kind or "issue") == "issue"
  end)
  local pr_label_raise = nil
  local fixing_raise = find_causal_raise(result, "devloop_fixing")
  t.is_true(comment_raise ~= nil)
  t.is_true(issue_label_raise ~= nil)
  t.is_true(fixing_raise ~= nil)
  pr_label_raise = find_raise(
    h.run_comment_handoff_from_request(
      comment_raise.payload,
      "IC_conflict_fixing_1",
      "observe-pr-conflict-fixing-handoff"
    ).raises,
    "github-proxy.github_issue_label_request",
    function(payload)
      return tostring(payload.target_kind or "") == "pr"
    end
  )
  t.is_true(pr_label_raise ~= nil)
  t.eq(issue_label_raise.payload.add_labels[1], "fkst-dev:fixing")
  t.eq(issue_label_raise.payload.label_colors["fkst-dev:fixing"], "D93F0B")
  t.eq(pr_label_raise.payload.add_labels[1], "fkst-dev:fixing")
  t.eq(pr_label_raise.payload.label_colors["fkst-dev:fixing"], "D93F0B")
  t.eq(pr_label_raise.payload.expected_proposal_id, proposal_id)
  t.eq(pr_label_raise.payload.expected_state, "fixing")
  t.eq(pr_label_raise.payload.expected_version, version .. "/fix/1")
  t.eq(fixing_raise.payload.proposal_id, proposal_id)
  t.eq(fixing_raise.payload.pr_number, 7)
  t.eq(fixing_raise.payload.version, version .. "/fix/1")
  t.eq(fixing_raise.payload.reviewed_head_sha, "def456")
  t.eq(fixing_raise.payload.gate_failure_excerpt, "mergeable-conflicting")
  t.is_true(tostring(fixing_raise.payload.review_proposal_id):find(expected_from_state, 1, true) == nil)
end

return {
  test_observe_pr_reviewing_conflict_redrives_to_fixing = function()
    mock_pr("reviewing", "CONFLICTING", "DIRTY")
    assert_conflict_redrive(run_observe_pr_mergeability("observe-pr-reviewing-conflict"), "reviewing")
  end,

  test_observe_pr_pr_open_conflict_redrives_to_fixing_before_reviewing = function()
    mock_pr("pr-open", "CONFLICTING", "DIRTY")
    local result = run_observe_pr_mergeability("observe-pr-pr-open-conflict")
    assert_conflict_redrive(result, "pr-open")
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
  end,

  test_observe_pr_fixing_conflict_replays_current_fixing_round = function()
    local fixing_version = version .. "/fix/1"
    local review_proposal_id = devloop_base.pr_review_proposal_id(repo, 7, version, "def456")
    local review_dedup_key = "observe-pr-conflict/" .. proposal_id .. "/" .. version .. "/7"
    local comments = {
      m_builders.pr_origin_marker(core, proposal_id, "42", branch, fixing_version, "dev"),
      core.state_marker(proposal_id, "fixing", fixing_version),
      m_builders.merge_gate_marker(core, 
        proposal_id,
        7,
        fixing_version,
        review_proposal_id,
        review_dedup_key,
        "def456",
        nil,
        "mergeable-conflicting"
      ),
    }
    entity_read_mocks.mock_pr_view_selector(t, {
      repo = repo,
      number = 7,
      comments = comments,
      head = branch,
      head_sha = "def456",
      base_branch = "dev",
      state = "OPEN",
      mergeable = "CONFLICTING",
      merge_state = "DIRTY",
    }, entity_read_mocks.pr_origin_selector)

    local result = run_observe_pr_mergeability("observe-pr-fixing-conflict")
    t.eq(result.exit_code, 0)
    local fixing_raise = find_causal_raise(result, "devloop_fixing")
    t.is_true(fixing_raise ~= nil)
    t.eq(fixing_raise.payload.version, fixing_version)
    t.eq(fixing_raise.payload.review_dedup_key, review_dedup_key)
  end,

  test_observe_pr_conflict_redrive_is_idempotent_when_fixing_marker_visible = function()
    local fix_version = version .. "/fix/1"
    local review_proposal_id = devloop_base.pr_review_proposal_id(repo, 7, version, "def456")
    mock_pr("reviewing", "CONFLICTING", "DIRTY", {
      m_builders.pr_origin_marker(core, proposal_id, "42", branch, fix_version, "dev"),
      core.state_marker(proposal_id, "fixing", fix_version),
      m_builders.merge_gate_marker(core, 
        proposal_id,
        7,
        fix_version,
        review_proposal_id,
        "observe-pr-conflict/" .. proposal_id .. "/" .. version .. "/7",
        "def456",
        nil,
        "mergeable-conflicting"
      ),
    })

    local result = run_observe_pr_mergeability("observe-pr-conflict-idempotent")
    t.eq(result.exit_code, 0)
    local fixing_raise = find_causal_raise(result, "devloop_fixing")
    t.is_true(fixing_raise ~= nil)
    t.eq(fixing_raise.payload.version, fix_version)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment_raise ~= nil)
    local pr_label_raise = find_raise(h.run_comment_handoff_from_request(
      comment_raise.payload,
      "IC_conflict_idempotent_fixing_1",
      "observe-pr-conflict-idempotent-handoff"
    ).raises, "github-proxy.github_issue_label_request", function(payload)
      return tostring(payload.target_kind or "") == "pr"
    end)
    t.is_true(pr_label_raise ~= nil)
    t.eq(pr_label_raise.payload.add_labels[1], "fkst-dev:fixing")
    t.eq(pr_label_raise.payload.label_colors["fkst-dev:fixing"], "D93F0B")
    t.eq(pr_label_raise.payload.expected_proposal_id, proposal_id)
    t.eq(pr_label_raise.payload.expected_state, "fixing")
    t.eq(pr_label_raise.payload.expected_version, fix_version)
  end,
}
