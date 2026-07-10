local devloop_base = require("devloop.base")
local payloads_builders = require("devloop.payloads.builders")
local ci_repair_attempts = require("core.ci_repair_attempts")
local ci_repair_retry = require("core.ci_repair_retry")
local config = require("devloop.config")
local contract_time = require("contract.time")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local m_builders = require("devloop.markers.builders")
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local repo = "owner/repo"
local pr_number = 7
local proposal_id = "github-devloop/issue/owner/repo/42"
local branch = "devloop-owner-repo-42-01HY"
local base_version = h.reviewing().version
local source_ref = {
  kind = "external",
  ref = "owner/repo#pr/7",
}
local fixed_now_seconds = contract_time.iso_timestamp_epoch_seconds("2026-06-03T02:00:00Z")

local function version_at(round)
  local version = base_version
  for _ = 1, round do
    version = core.next_fix_version(version)
  end
  return version
end

local function failure_key(suffix)
  return "head:def456/checks:digest-" .. tostring(suffix)
end

local function rollup(conclusion, head_sha)
  return '[{"__typename":"CheckRun","completedAt":"2026-06-03T02:04:04Z","conclusion":"'
    .. tostring(conclusion)
    .. '","detailsUrl":"https://example.invalid/checks/test","name":"test","startedAt":"2026-06-03T02:03:04Z","status":"COMPLETED","workflowName":"test","headSha":"'
    .. tostring(head_sha)
    .. '"}]'
end

local function attempt_fixture(round, fields)
  local selected = fields or {}
  local version = selected.version or version_at(round)
  local reviewed_head_sha = selected.reviewed_head_sha or "def456"
  local ci_failure_key = selected.ci_failure_key or failure_key("0000000101")
  local review_proposal_id = devloop_base.pr_review_proposal_id(repo, pr_number, base_version, reviewed_head_sha)
  local review_dedup_key = "consensus:" .. review_proposal_id .. "/review"
  local fix = payloads_builders.build_devloop_fixing_payload({
    proposal_id = proposal_id,
    impl_version = version,
  }, pr_number, {
    review_proposal_id = review_proposal_id,
    review_dedup_key = review_dedup_key,
    reviewed_head_sha = reviewed_head_sha,
    gate_baseline_sha = selected.gate_baseline_sha or "abc123",
    ci_failure_key = ci_failure_key,
    gate_failure_excerpt = "own-ci-red",
  }, source_ref)
  local state_marker = core.state_marker(proposal_id, "fixing", version)
  if selected.state_marker_created_at ~= nil then
    state_marker = {
      body = state_marker,
      author_login = "fkst-test-bot",
      created_at = selected.state_marker_created_at,
    }
  end
  local comments = {
    m_builders.pr_origin_marker(proposal_id, "42", branch, base_version, "dev"),
    state_marker,
    m_builders.merge_gate_marker(
      proposal_id,
      pr_number,
      version,
      review_proposal_id,
      review_dedup_key,
      reviewed_head_sha,
      selected.gate_baseline_sha or "abc123",
      "own-ci-red",
      nil,
      ci_failure_key
    ),
  }
  if selected.with_attempt ~= false then
    table.insert(comments, {
      body = ci_repair_attempts.comment_request(repo, fix, "no-fix", "No repaired revision was published.").body,
      author_login = "fkst-test-bot",
      created_at = selected.attempt_created_at or "2026-06-03T01:00:00Z",
    })
  end
  return {
    fix = fix,
    comments = comments,
    version = version,
    review_proposal_id = review_proposal_id,
    review_dedup_key = review_dedup_key,
  }
end

local function pr_event(name)
  return {
    schema = "github-proxy.v1",
    type = "pr",
    repo = repo,
    number = pr_number,
    state = "OPEN",
    updated_at = "2026-06-04T01:02:03Z",
    dedup_key = "owner/repo#pr#7@" .. tostring(name),
    source_ref = source_ref,
  }
end

local function mock_retry_pr(fixture, fields)
  local selected = fields or {}
  local head_sha = selected.head_sha or "def456"
  local common = {
    repo = repo,
    number = pr_number,
    comments = fixture.comments,
    head = branch,
    head_sha = head_sha,
    base_branch = "dev",
    base_sha = selected.base_sha or "abc123",
    state = "OPEN",
    labels = { "fkst-dev:fixing" },
    mergeable = "MERGEABLE",
    merge_state = "UNSTABLE",
    status_check_rollup_json = rollup(selected.conclusion or "FAILURE", head_sha),
    register_all_views = true,
    register_merge_views = false,
    times = 12,
  }
  entity_read_mocks.mock_pr_read_forms(t, common)
  entity_read_mocks.mock_pr_merge_view(t, {
    repo = repo,
    number = pr_number,
    comments = fixture.comments,
    head = branch,
    head_sha = selected.reobserved_head_sha or head_sha,
    base_branch = "dev",
    base_sha = selected.base_sha or "abc123",
    state = "OPEN",
    labels = { "fkst-dev:fixing" },
    mergeable = "MERGEABLE",
    merge_state = "UNSTABLE",
    status_check_rollup_json = rollup(
      selected.conclusion or "FAILURE",
      selected.reobserved_head_sha or head_sha
    ),
  }, 1)
  if selected.conclusion ~= "SUCCESS" then
    h.mock_required_check_runs_for(selected.reobserved_head_sha or head_sha, "failure", repo)
  end
end

local function run_retry(fixture, name, fields)
  mock_retry_pr(fixture, fields)
  return h.run_observe_pr(pr_event(name), opts(name), fields and fields.now_seconds or fixed_now_seconds)
end

local function state_comment(result, state_name)
  return h.find_raise(result.raises, "github-proxy.github_pr_comment_request", function(payload)
    return type(payload.handoff) == "table" and payload.handoff.kind == "github-devloop." .. tostring(state_name)
  end)
end

local function has_blocked_state_marker(result)
  return h.find_raise(result.raises, "github-proxy.github_pr_comment_request", function(payload)
    return tostring(payload.body or ""):find('state="blocked"', 1, true) ~= nil
  end) ~= nil
end

local function assert_reviewing_without_terminal(result, expected_version)
  t.eq(result.exit_code, 0)
  local reviewing = h.find_causal_raise(result, "devloop_reviewing")
  t.is_true(reviewing ~= nil)
  t.eq(reviewing.payload.version, expected_version)
  t.eq(h.find_raise(result.raises, "devloop_fix_reconcile"), nil)
  t.eq(has_blocked_state_marker(result), false)
  t.eq(h.count_calls("codex"), 0)
end

local function run_own_ci_reconcile(reconcile, comments, name, fields)
  local selected = fields or {}
  local head_sha = selected.head_sha or reconcile.bound_head_sha
  local conclusion = selected.conclusion or "FAILURE"
  h.mock_default_issue_claim()
  entity_read_mocks.mock_pr_merge_view(t, {
    repo = repo,
    number = pr_number,
    comments = comments,
    head = branch,
    head_sha = head_sha,
    base_branch = "dev",
    base_sha = "abc123",
    state = "OPEN",
    labels = { "fkst-dev:fixing" },
    mergeable = "MERGEABLE",
    merge_state = "UNSTABLE",
    status_check_rollup_json = rollup(conclusion, head_sha),
  }, 4)
  if conclusion ~= "SUCCESS" and tostring(head_sha) == tostring(reconcile.bound_head_sha) then
    h.mock_required_check_runs_for(head_sha, "failure", repo)
  end
  return h.run_department("departments/reconcile/main.lua", {
    queue = "devloop_fix_reconcile",
    payload = reconcile,
  }, opts(name))
end

return {
  test_completed_round_before_backoff_due_defers_without_codex_or_timeout_mutation = function()
    local current_seconds = fixed_now_seconds
    local fixture = attempt_fixture(2, {
      attempt_created_at = "2026-06-03T01:50:01Z",
    })
    local emitted = {}
    local original_raise = raise
    raise = function(queue, payload)
      table.insert(emitted, { queue = queue, payload = payload })
    end
    local ok, decision = pcall(function()
      return ci_repair_retry.evaluate(core, {
        state = "fixing",
        version = fixture.version,
      }, {
        dept = "observe_pr",
        repo = repo,
        proposal_id = proposal_id,
        pr_number = pr_number,
        review_proposal_id = fixture.review_proposal_id,
        review_dedup_key = fixture.review_dedup_key,
        reviewed_head_sha = "def456",
        source_ref = source_ref,
        comments = fixture.comments,
        now_seconds = current_seconds,
      })
    end)
    raise = original_raise
    if not ok then
      error(decision)
    end

    t.eq(decision.kind, "defer")
    t.eq(decision.completed_seconds, contract_time.iso_timestamp_epoch_seconds("2026-06-03T01:50:01Z"))
    t.eq(decision.delay_seconds, 2 * 300)
    t.eq(decision.due_seconds, contract_time.iso_timestamp_epoch_seconds("2026-06-03T02:00:01Z"))
    t.eq(#emitted, 0)
    t.eq(h.count_calls("codex"), 0)
  end,

  test_completed_round_exactly_at_backoff_deadline_admits_next_fix_generation = function()
    local fixture = attempt_fixture(2, { attempt_created_at = "2026-06-03T01:50:00Z" })
    local result = run_retry(fixture, "ci-repair-at-backoff-deadline")

    t.eq(result.exit_code, 0)
    local fixing = h.find_causal_raise(result, "devloop_fixing")
    t.is_true(fixing ~= nil)
    t.eq(fixing.payload.version, version_at(3))
    t.eq(core.version_fix_round(fixing.payload.version), 3)
    t.eq(fixing.payload.repair_input, "ci-failure")
    t.eq(state_comment(result, "blocked"), nil)
  end,

  test_completed_round_just_after_backoff_deadline_admits_next_fix_generation = function()
    local fixture = attempt_fixture(2, { attempt_created_at = "2026-06-03T01:49:59Z" })
    local result = run_retry(fixture, "ci-repair-after-backoff-deadline")

    t.eq(result.exit_code, 0)
    local fixing = h.find_causal_raise(result, "devloop_fixing")
    t.is_true(fixing ~= nil)
    t.eq(fixing.payload.version, version_at(3))
    t.eq(state_comment(result, "blocked"), nil)
  end,

  test_policy_invalid_time_reaches_blocked_terminal_with_structured_why = function()
    local round = 2
    local fixture = attempt_fixture(round, {
      version = version_at(round),
      state_marker_created_at = "not-a-timestamp",
    })
    local emitted = {}
    local original_raise = raise
    raise = function(queue, payload)
      table.insert(emitted, { queue = queue, payload = payload })
    end
    local ok, decision = pcall(function()
      return ci_repair_retry.evaluate(core, {
        state = "fixing",
        version = fixture.version,
        marker_created_at = "not-a-timestamp",
      }, {
        dept = "observe_pr",
        repo = repo,
        proposal_id = proposal_id,
        pr_number = pr_number,
        review_proposal_id = fixture.review_proposal_id,
        review_dedup_key = fixture.review_dedup_key,
        reviewed_head_sha = "def456",
        source_ref = source_ref,
        comments = fixture.comments,
        now_seconds = fixed_now_seconds,
      })
    end)
    raise = original_raise
    if not ok then error(decision) end
    t.eq(decision.kind, "terminate")
    local reconcile
    for _, item in ipairs(emitted) do
      if item.queue == "devloop_fix_reconcile" then reconcile = item end
    end
    t.is_true(reconcile ~= nil)
    t.eq(reconcile.payload.reason_class, "ci-repair-retry-policy-invalid")
    t.eq(reconcile.payload.bound_head_sha, "def456")

    local terminal = run_own_ci_reconcile(
      reconcile.payload,
      fixture.comments,
      "ci-repair-policy-invalid-terminal"
    )
    t.eq(terminal.exit_code, 0)
    local marker = h.find_raise(terminal.raises, "github-proxy.github_pr_comment_request", function(payload)
      return tostring(payload.body or ""):find('state="blocked"', 1, true) ~= nil
    end)
    t.is_true(marker ~= nil)
    t.is_true(marker.payload.body:find("ci-repair-retry-policy-invalid", 1, true) ~= nil)
  end,

  test_fresh_ci_failure_key_is_diagnostic_and_does_not_reset_round_budget = function()
    local old_key = failure_key("0000000202")
    local fixture = attempt_fixture(3, { ci_failure_key = old_key })
    local result = run_retry(fixture, "ci-repair-fresh-key")

    t.eq(result.exit_code, 0)
    local fixing = h.find_causal_raise(result, "devloop_fixing")
    t.is_true(fixing ~= nil)
    t.eq(fixing.payload.version, version_at(4))
    t.is_true(fixing.payload.ci_failure_key ~= old_key)
    t.eq(core.version_fix_round(fixing.payload.version), 4)
  end,

  test_same_head_green_routes_to_bumped_reviewing_without_repair_codex = function()
    local fixture = attempt_fixture(2)
    local result = run_retry(fixture, "ci-repair-green", { conclusion = "SUCCESS" })

    t.eq(result.exit_code, 0)
    local reviewing = h.find_causal_raise(result, "devloop_reviewing")
    t.is_true(reviewing ~= nil)
    t.eq(reviewing.payload.version, version_at(3))
    t.eq(state_comment(result, "fixing"), nil)
    t.eq(h.count_calls("codex"), 0)
  end,

  test_reobserved_head_advanced_routes_to_reviewing_without_terminal = function()
    local fixture = attempt_fixture(2)
    local result = run_retry(fixture, "ci-repair-head-advanced", {
      reobserved_head_sha = "feedface",
    })

    assert_reviewing_without_terminal(result, version_at(3))
  end,

  test_reobserved_head_advanced_at_cap_routes_to_reviewing_without_terminal = function()
    local fixture = attempt_fixture(config.max_fix_rounds())
    local result = run_retry(fixture, "ci-repair-head-advanced-at-cap", {
      reobserved_head_sha = "feedface",
      now_seconds = fixed_now_seconds + config.liveness_poll_cadence_seconds(),
    })

    assert_reviewing_without_terminal(result, version_at(config.max_fix_rounds() + 1))
  end,

  test_same_head_green_at_cap_routes_to_reviewing_without_terminal = function()
    local fixture = attempt_fixture(config.max_fix_rounds())
    local result = run_retry(fixture, "ci-repair-green-at-cap", {
      conclusion = "SUCCESS",
      now_seconds = fixed_now_seconds + config.liveness_poll_cadence_seconds(),
    })

    assert_reviewing_without_terminal(result, version_at(config.max_fix_rounds() + 1))
  end,

  test_cap_exhaustion_reaches_existing_fix_reconcile_terminal_marker = function()
    local fixture = attempt_fixture(config.max_fix_rounds())
    local admitted = run_retry(fixture, "ci-repair-cap", {
      now_seconds = fixed_now_seconds + config.liveness_poll_cadence_seconds(),
    })

    if admitted.exit_code ~= 0 then
      error("fix-round termination admission failed: "
        .. tostring(admitted.error or admitted.stderr or admitted.output or admitted.message or "unknown failure"))
    end
    local reconcile = h.find_raise(admitted.raises, "devloop_fix_reconcile")
    local decompose = h.find_raise(admitted.raises, "github-devloop-decompose.devloop_decompose")
    t.is_true(reconcile ~= nil)
    t.is_true(decompose ~= nil)
    t.eq(reconcile.payload.round, config.max_fix_rounds())
    t.eq(reconcile.payload.reason_class, "fix-loop-max-rounds")
    t.eq(reconcile.payload.bound_head_sha, "def456")
    t.eq(decompose.payload.round, config.max_fix_rounds())
    t.eq(state_comment(admitted, "fixing"), nil)
    t.eq(h.count_calls("codex"), 0)

    local terminal = run_own_ci_reconcile(reconcile.payload, fixture.comments, "ci-repair-cap-terminal")
    if terminal.exit_code ~= 0 then
      error("fix reconcile failed: "
        .. tostring(terminal.error or terminal.stderr or terminal.output or terminal.message or "unknown failure"))
    end
    local marker = h.find_raise(terminal.raises, "github-proxy.github_pr_comment_request", function(payload)
      return tostring(payload.body or ""):find(
        core.state_marker(reconcile.payload.proposal_id, "blocked", reconcile.payload.issue_version),
        1,
        true
      ) ~= nil
    end)
    t.is_true(marker ~= nil)
    t.is_true(marker.payload.body:find("fix-loop-max-rounds-after-" .. tostring(config.max_fix_rounds()) .. "-rounds", 1, true) ~= nil)
  end,

  test_cap_reconcile_head_advance_between_emit_and_consume_does_not_block = function()
    local fixture = attempt_fixture(config.max_fix_rounds())
    local emitted = run_retry(fixture, "ci-repair-cap-head-race", {
      now_seconds = fixed_now_seconds + config.liveness_poll_cadence_seconds(),
    })
    local reconcile = h.find_raise(emitted.raises, "devloop_fix_reconcile").payload

    local consumed = run_own_ci_reconcile(reconcile, fixture.comments, "ci-repair-cap-head-race-consume", {
      head_sha = "feedface",
    })
    if consumed.exit_code ~= 0 then
      error("head-race reconcile failed: " .. tostring(consumed.error or consumed.stderr or consumed.output or consumed.message or "unknown failure"))
    end
    t.eq(consumed.exit_code, 0)
    t.eq(has_blocked_state_marker(consumed), false)
    t.eq(#consumed.raises, 0)
  end,

  test_cap_reconcile_same_head_green_between_emit_and_consume_does_not_block = function()
    local fixture = attempt_fixture(config.max_fix_rounds())
    local emitted = run_retry(fixture, "ci-repair-cap-green-race", {
      now_seconds = fixed_now_seconds + config.liveness_poll_cadence_seconds(),
    })
    local reconcile = h.find_raise(emitted.raises, "devloop_fix_reconcile").payload

    local consumed = run_own_ci_reconcile(reconcile, fixture.comments, "ci-repair-cap-green-race-consume", {
      conclusion = "SUCCESS",
    })
    if consumed.exit_code ~= 0 then
      error("green-race reconcile failed: " .. tostring(consumed.error or consumed.stderr or consumed.output or consumed.message or "unknown failure"))
    end
    t.eq(consumed.exit_code, 0)
    t.eq(has_blocked_state_marker(consumed), false)
    t.eq(#consumed.raises, 0)
  end,

  test_two_supervisors_and_base_churn_choose_one_next_generation_identity = function()
    local first_fixture = attempt_fixture(2, { gate_baseline_sha = "abc123" })
    local first = run_retry(first_fixture, "ci-repair-supervisor-a", { base_sha = "abc123" })
    local first_request = state_comment(first, "fixing")
    t.is_true(first_request ~= nil)

    local second_fixture = attempt_fixture(2, { gate_baseline_sha = "ba5e9999" })
    local second = run_retry(second_fixture, "ci-repair-supervisor-b", { base_sha = "ba5e9999" })
    local second_request = state_comment(second, "fixing")
    t.is_true(second_request ~= nil)

    t.eq(first_request.payload.handoff.version, version_at(3))
    t.eq(second_request.payload.handoff.version, version_at(3))
    t.eq(first_request.payload.dedup_key, second_request.payload.dedup_key)
    t.eq(first_request.payload.handoff.dedup_key, second_request.payload.handoff.dedup_key)
    t.eq(
      payloads_builders.fixing_work_unit_key(first_request.payload.handoff),
      payloads_builders.fixing_work_unit_key(second_request.payload.handoff)
    )
    t.eq(h.count_calls("codex"), 0)
  end,
}
