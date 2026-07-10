local devloop_base = require("devloop.base")
local config = require("devloop.config")
local fix_rounds = require("core.fix_rounds")
local replay_fields = require("devloop.replay_fields")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts

local repo = "owner/repo"
local branch = "devloop-owner-repo-42-01HY"
local proposal_id = "github-devloop/issue/owner/repo/42"
local pr_number = 7
local base_version = h.reviewing().version

local function version_at(round)
  local version = base_version
  for _ = 1, round do
    version = core.next_fix_version(version)
  end
  return version
end

local function merge_event(version, head_sha)
  local review_proposal_id = devloop_base.pr_review_proposal_id(repo, pr_number, version, head_sha)
  return h.merge_ready({
    proposal_id = proposal_id,
    pr_number = pr_number,
    version = version,
    reviewed_head_sha = head_sha,
    review_proposal_id = review_proposal_id,
    review_dedup_key = "consensus:" .. review_proposal_id .. "/review",
  })
end

local function run_own_ci_merging_replay(version, head_sha, name)
  local event = merge_event(version, head_sha)
  local comments = {}
  for index, body in ipairs(h.merge_comments_with_merging(event, branch, version)) do
    table.insert(comments, {
      id = tostring(name) .. ":" .. tostring(index),
      body = body,
      author_login = "fkst-test-bot",
      created_at = "2026-06-03T01:00:00Z",
    })
  end
  local current_pr = {
    number = pr_number,
    comments = comments,
    head_ref_name = branch,
    head_sha = head_sha,
    base_ref_name = "dev",
    base_ref_oid = "abc123",
    state = "OPEN",
    mergeable = "MERGEABLE",
    merge_state_status = "CLEAN",
    status_check_rollup_present = true,
    status_check_rollup = {},
  }
  local state = {
    state = "merging",
    version = version,
    proposal_id = proposal_id,
    marker_created_at = "2026-06-03T01:00:00Z",
  }
  local facts = {
    proposal_id = proposal_id,
    state = state,
    source_ref = h.pr_source_ref(),
    link = {
      proposal_id = proposal_id,
      pr_number = pr_number,
      branch = branch,
      impl_version = version,
      base_branch = "dev",
    },
    snapshot = {
      comments = comments,
      state = state,
      prs = { { number = pr_number, current = current_pr } },
    },
  }
  local emitted = {}
  local original_raise = raise
  local original_evaluate = core.evaluate_ci_status_gate
  raise = function(queue, payload)
    table.insert(emitted, { queue = queue, payload = payload })
  end
  core.evaluate_ci_status_gate = function()
    return false, "own-ci-red"
  end
  entity_read_mocks.mock_pr_merge_view(t, {
    repo = repo,
    number = pr_number,
    comments = comments,
    head = branch,
    head_sha = head_sha,
    base_branch = "dev",
    base_sha = "abc123",
    head_repo = repo,
    state = "OPEN",
    mergeable = "MERGEABLE",
    merge_state = "CLEAN",
    status_check_rollup_json = '[{"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"FAILURE","headSha":"'
      .. tostring(head_sha)
      .. '"}]',
  })
  h.mock_required_check_runs_for(head_sha, "failure", repo)
  local tools = {
    find_linked_pr = function(snapshot, number)
      for _, item in ipairs(snapshot and snapshot.prs or {}) do
        if tostring(item.number or "") == tostring(number or "") then
          return item.current
        end
      end
      return nil
    end,
    log_skip = function()
      return false
    end,
    raise_effects = function(_dept, _proposal_id, _state, _version, _labels, effects)
      for _, effect in ipairs(effects or {}) do
        table.insert(emitted, effect)
      end
      return true
    end,
  }
  local replayers = core.replayer_review_registry(tools)
  local row = replay_fields.restart_transition_row(core.restart_transition_table(), "merging")
  local ok, outcome = pcall(
    replayers.merging,
    "observe_pr",
    { repo = repo, number = 42, source_ref = h.source_ref() },
    state,
    row,
    facts
  )
  raise = original_raise
  core.evaluate_ci_status_gate = original_evaluate
  if not ok then
    error(outcome)
  end
  return { raises = emitted, outcome = outcome, comments = comments }, event
end

local function with_counted_owner(fn)
  local calls = 0
  local original = fix_rounds.admit_own_ci_continuation
  fix_rounds.admit_own_ci_continuation = function(...)
    calls = calls + 1
    return original(...)
  end
  local ok, result, event = pcall(fn)
  fix_rounds.admit_own_ci_continuation = original
  if not ok then
    error(result)
  end
  return calls, result, event
end

local function fixing_handoff(result)
  return h.find_raise(result.raises, "github-proxy.github_pr_comment_request", function(payload)
    return type(payload.handoff) == "table" and payload.handoff.kind == "github-devloop.fixing"
  end)
end

local function consume_own_ci_reconcile(reconcile, comments, name)
  h.mock_default_issue_claim()
  entity_read_mocks.mock_pr_merge_view(t, {
    repo = repo,
    number = pr_number,
    comments = comments,
    head = branch,
    head_sha = reconcile.bound_head_sha,
    base_branch = "dev",
    base_sha = "abc123",
    state = "OPEN",
    labels = { "fkst-dev:fixing" },
    mergeable = "MERGEABLE",
    merge_state = "UNSTABLE",
    status_check_rollup_json = '[{"__typename":"CheckRun","completedAt":"2026-06-03T02:04:04Z","conclusion":"FAILURE","detailsUrl":"https://example.invalid/checks/test","name":"test","startedAt":"2026-06-03T02:03:04Z","status":"COMPLETED","workflowName":"test","headSha":"'
      .. tostring(reconcile.bound_head_sha)
      .. '"}]',
  }, 4)
  h.mock_required_check_runs_for(reconcile.bound_head_sha, "failure", repo)
  return h.run_department("departments/reconcile/main.lua", {
    queue = "devloop_fix_reconcile",
    payload = reconcile,
  }, opts(name))
end

return {
  test_merging_replay_at_cap_uses_owner_and_emits_terminal_without_mint = function()
    local calls, result, event = with_counted_owner(function()
      return run_own_ci_merging_replay(
        version_at(config.max_fix_rounds()),
        "def456",
        "merging-replay-own-ci-at-cap"
      )
    end)
    t.eq(calls, 1)
    t.eq(fixing_handoff(result), nil)
    t.eq(h.find_causal_raise(result, "devloop_fixing"), nil)
    local reconcile = h.find_raise(result.raises, "devloop_fix_reconcile")
    local decompose = h.find_raise(result.raises, "github-devloop-decompose.devloop_decompose")
    t.is_true(reconcile ~= nil)
    t.is_true(decompose ~= nil)
    t.eq(reconcile.payload.issue_version, event.version)
    t.eq(reconcile.payload.bound_head_sha, event.reviewed_head_sha)
    t.eq(reconcile.payload.round, config.max_fix_rounds())
    t.eq(reconcile.payload.reason_class, "fix-loop-max-rounds")

    local terminal = consume_own_ci_reconcile(
      reconcile.payload,
      result.comments,
      "merging-replay-own-ci-at-cap-terminal"
    )
    t.eq(terminal.exit_code, 0)
    local marker = h.find_raise(terminal.raises, "github-proxy.github_pr_comment_request", function(payload)
      return tostring(payload.body or ""):find(
        core.state_marker(reconcile.payload.proposal_id, "blocked", reconcile.payload.issue_version),
        1,
        true
      ) ~= nil
    end)
    t.is_true(marker ~= nil)
    t.is_true(marker.payload.body:find(
      "fix-loop-max-rounds-after-" .. tostring(config.max_fix_rounds()) .. "-rounds",
      1,
      true
    ) ~= nil)
  end,

  test_merging_replay_below_cap_admits_exactly_one_generation_through_owner = function()
    local calls, result = with_counted_owner(function()
      return run_own_ci_merging_replay(version_at(1), "def456", "merging-replay-own-ci-below-cap")
    end)
    t.eq(calls, 1)
    local handoff = fixing_handoff(result)
    t.is_true(handoff ~= nil)
    t.eq(handoff.payload.handoff.version, version_at(2))
    t.eq(core.version_fix_round(handoff.payload.handoff.version), 2)
    t.eq(h.find_raise(result.raises, "devloop_fix_reconcile"), nil)
  end,

  test_head_review_ping_pong_admits_only_bounded_own_ci_generations = function()
    local version = base_version
    local head_sha = "def456"
    local admitted_generations = 0
    local owner_calls = 0
    local terminated = false
    for cycle = 1, config.max_fix_rounds() * 2 do
      local calls, result = with_counted_owner(function()
        return run_own_ci_merging_replay(version, head_sha, "merging-replay-ping-pong-" .. tostring(cycle))
      end)
      owner_calls = owner_calls + calls
      local terminal = h.find_raise(result.raises, "devloop_fix_reconcile")
      if terminal ~= nil then
        t.eq(terminal.payload.issue_version, version)
        t.eq(terminal.payload.round, config.max_fix_rounds())
        terminated = true
        break
      end

      local handoff = fixing_handoff(result)
      t.is_true(handoff ~= nil)
      admitted_generations = admitted_generations + 1
      local fixing_version = handoff.payload.handoff.version
      t.eq(core.version_fix_round(fixing_version), core.version_fix_round(version) + 1)

      -- Production's successful fixing exit pushes a new head and performs its load-bearing
      -- fixing -> reviewing bump. The approved review preserves that version into merging,
      -- where the next own-CI-red head must pass the same admission owner again.
      version = core.next_fix_version(fixing_version)
      head_sha = head_sha == "def456" and "feedface" or "def456"
    end
    t.eq(terminated, true)
    t.is_true(admitted_generations <= config.max_fix_rounds())
    t.eq(core.version_fix_round(version), config.max_fix_rounds())
    t.eq(owner_calls, admitted_generations + 1)
  end,
}
