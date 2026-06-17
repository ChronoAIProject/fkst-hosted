local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local repo = "owner/repo"
local proposal_id = "github-devloop/issue/owner/repo/42"
local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local head_sha = "def456"

local lineages = {
  fix = version .. "/fix/1",
  review_loop = version .. "/review-loop/2",
}

local function trusted_comment(body, created_at)
  return {
    body = body,
    author_login = "fkst-test-bot",
    created_at = created_at,
  }
end

local function state_comment(state_name, state_version, created_at)
  return trusted_comment(core.state_marker(proposal_id, state_name, state_version), created_at)
end

local function converge_round_comment(created_at)
  local source_ref = core.issue_source_ref(repo, 42)
  return trusted_comment(core.converge_round_marker(
    proposal_id,
    version,
    core.source_ref_digest(source_ref),
    1,
    "consensus:" .. proposal_id .. "/loop/1",
    "Still thinking",
    { { angle = "minimal", verdict = "continue", digest = "recent" } }
  ), created_at)
end

local function merge_gate_wait_comment(state_version, created_at)
  return trusted_comment(core.merge_gate_wait_marker(proposal_id, 7, state_version, head_sha, "ci-wait", "CI_WAIT"), created_at)
end

local function timeout_attempt_comment(state_name, state_version, round, source_ref)
  return trusted_comment(core.timeout_attempt_marker(proposal_id, state_version, state_name, round, source_ref), "2026-06-03T00:00:00Z")
end

local function capture_raises(fn)
  local raised = {}
  local original_log_raise = core.log_raise
  core.log_raise = function(_, _, queue, payload)
    table.insert(raised, { queue = queue, payload = payload })
  end
  local ok, err = pcall(fn)
  core.log_raise = original_log_raise
  if not ok then
    error(err)
  end
  return raised
end

local function old_merge_state(state_name, state_version)
  return {
    state = state_name,
    version = state_version,
    proposal_id = proposal_id,
    marker_created_at = "2026-06-03T00:00:00Z",
  }
end

local function recent_merge_state(state_name, state_version)
  return {
    state = state_name,
    version = state_version,
    proposal_id = proposal_id,
    marker_created_at = "2026-06-04T00:00:00Z",
  }
end

local function merge_timeout_facts(pr_comments, now_seconds)
  return {
    proposal_id = proposal_id,
    source_ref = core.pr_source_ref(repo, 7),
    current = { comments = {} },
    current_pr = {
      head_sha = head_sha,
      comments = pr_comments or {},
    },
    head_sha = head_sha,
    now_seconds = now_seconds,
  }
end

local function assert_fresh_merge_wait_does_not_extend_absolute_cap(state_name, lineage_version)
  local row = core.restart_transition_row(state_name)
  local now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z")
  local wait_lineage = lineage_version or version
  local timeout_version = wait_lineage .. "/timeout/" .. state_name .. "/3"
  local wait = merge_gate_wait_comment(wait_lineage, "2026-06-04T00:30:00Z")
  local due, age = core.liveness_timeout_due_with_facts(
    row,
    old_merge_state(state_name, timeout_version),
    merge_timeout_facts({ wait }, now_seconds),
    now_seconds
  )
  t.eq(due, true)
  t.eq(age, 1502)

  local raised = capture_raises(function()
    local applied = core.maybe_timeout_redrive_from_table("liveness_scan", {
      repo = repo,
      number = 42,
      source_ref = core.issue_source_ref(repo, 42),
    }, old_merge_state(state_name, timeout_version), row, merge_timeout_facts({ wait }, now_seconds))
    t.eq(applied, true)
  end)
  t.eq(#raised, 1)
  t.eq(raised[1].queue, "devloop_timeout_reconcile")
  t.eq(raised[1].payload.state, state_name)
  t.eq(raised[1].payload.issue_version, timeout_version)
  t.eq(raised[1].payload.round, 3)
end

local function assert_fresh_merge_wait_defers_within_absolute_cap(state_name)
  local row = core.restart_transition_row(state_name)
  local now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z")
  local wait = merge_gate_wait_comment(version, "2026-06-04T00:30:00Z")
  local due, age = core.liveness_timeout_due_with_facts(
    row,
    recent_merge_state(state_name, version),
    merge_timeout_facts({ wait }, now_seconds),
    now_seconds
  )
  t.eq(due, false)
  t.eq(age, 62)

  local raised = capture_raises(function()
    local applied = core.maybe_timeout_redrive_from_table("liveness_scan", {
      repo = repo,
      number = 42,
      source_ref = core.issue_source_ref(repo, 42),
    }, recent_merge_state(state_name, version), row, merge_timeout_facts({ wait }, now_seconds))
    t.eq(applied, true)
  end)
  t.eq(#raised, 0)
end

local function assert_stale_or_missing_merge_wait_escalates(state_name, wait_comment, lineage_version)
  local row = core.restart_transition_row(state_name)
  local now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z")
  local wait_lineage = lineage_version or version
  local timeout_version = wait_lineage .. "/timeout/" .. state_name .. "/3"
  local raised = capture_raises(function()
    local applied = core.maybe_timeout_redrive_from_table("liveness_scan", {
      repo = repo,
      number = 42,
      source_ref = core.issue_source_ref(repo, 42),
    }, old_merge_state(state_name, timeout_version), row, merge_timeout_facts(wait_comment and { wait_comment } or {}, now_seconds))
    t.eq(applied, true)
  end)
  t.eq(#raised, 1)
  t.eq(raised[1].queue, "devloop_timeout_reconcile")
  t.eq(raised[1].payload.state, state_name)
  t.eq(raised[1].payload.issue_version, timeout_version)
  t.eq(raised[1].payload.round, 3)
end

local function assert_stale_merge_wait_falls_back_to_under_budget_state_age(state_name)
  local row = core.restart_transition_row(state_name)
  local now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z")
  local stale_wait = merge_gate_wait_comment(version, "2026-06-03T00:00:00Z")
  local due, age = core.liveness_timeout_due_with_facts(
    row,
    recent_merge_state(state_name, version),
    merge_timeout_facts({ stale_wait }, now_seconds),
    now_seconds
  )
  t.eq(due, false)
  t.eq(age, 62)

  local raised = capture_raises(function()
    local applied = core.maybe_timeout_redrive_from_table("liveness_scan", {
      repo = repo,
      number = 42,
      source_ref = core.issue_source_ref(repo, 42),
    }, recent_merge_state(state_name, version), row, merge_timeout_facts({ stale_wait }, now_seconds))
    t.eq(applied, false)
  end)
  t.eq(#raised, 0)
end

local function run_timeout_reconcile(payload, comments, name)
  local source_repo, source_pr = core.parse_pr_source_ref(payload and payload.source_ref)
  local common_issue = {
    repo = repo,
    number = 42,
    title = "Issue 42",
    body = "",
    state = "OPEN",
    updated_at = "2026-06-04T01:02:03Z",
    labels = { "fkst-dev:thinking" },
    comments = comments,
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
    register_all_views = true,
    times = 1,
  }
  entity_read_mocks.mock_issue_read_forms(t, common_issue)
  if source_pr ~= nil then
    entity_read_mocks.mock_pr_read_forms(t, {
      repo = source_repo or repo,
      number = source_pr,
      head_sha = head_sha,
      comments = comments,
      state = "OPEN",
      register_all_views = true,
      times = 1,
    })
  end
  return t.run_department("departments/reconcile/main.lua", {
    queue = "devloop_timeout_reconcile",
    payload = payload,
  }, opts(name or "liveness-timeout-clock"))
end

return {
  test_live_defer_timeout_clock_uses_fresh_thinking_heartbeat = function()
    local row = core.restart_transition_row("thinking")
    local state = {
      state = "thinking",
      version = version,
      proposal_id = proposal_id,
      marker_created_at = "2026-06-03T00:00:00Z",
    }
    local comments = {
      state_comment("thinking", version, "2026-06-03T00:00:00Z"),
      converge_round_comment(os.date("!%Y-%m-%dT%H:%M:%SZ", now() - 60)),
    }
    local due, age = core.liveness_timeout_due_with_facts(row, state, {
      proposal_id = proposal_id,
      source_ref = core.issue_source_ref(repo, 42),
      current = { comments = comments },
      now_seconds = now(),
    }, now())
    t.eq(due, false)
    t.is_true(age ~= nil and age < 2)
  end,

  test_timeout_reconcile_why_reports_effective_heartbeat_age = function()
    local timeout_version = version .. "/timeout/thinking/3"
    local source_ref = core.issue_source_ref(repo, 42)
    local payload = core.build_devloop_timeout_reconcile_payload(core.restart_transition_row("thinking"), {
      state = "thinking",
      version = timeout_version,
    }, proposal_id, source_ref, 3)
    local heartbeat_created = os.date("!%Y-%m-%dT%H:%M:%SZ", now() - 121 * 60)
    local result = run_timeout_reconcile(payload, {
      state_comment("thinking", timeout_version, "2026-06-03T00:00:00Z"),
      timeout_attempt_comment("thinking", version, 1, source_ref),
      timeout_attempt_comment("thinking", version, 2, source_ref),
      converge_round_comment(heartbeat_created),
    }, "timeout-reconcile-heartbeat-age")
    t.eq(result.exit_code, 0)
    local comment = h.find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment ~= nil)
    t.is_true(comment.payload.body:find("age_minutes=121", 1, true) ~= nil)
  end,

  test_merge_ready_fresh_merge_gate_wait_past_absolute_cap_escalates = function()
    assert_fresh_merge_wait_does_not_extend_absolute_cap("merge-ready")
  end,

  test_merging_fresh_merge_gate_wait_past_absolute_cap_escalates = function()
    assert_fresh_merge_wait_does_not_extend_absolute_cap("merging")
  end,

  test_merge_ready_fresh_merge_gate_wait_past_absolute_cap_escalates_fix_lineage = function()
    assert_fresh_merge_wait_does_not_extend_absolute_cap("merge-ready", lineages.fix)
  end,

  test_merge_ready_fresh_merge_gate_wait_past_absolute_cap_escalates_review_loop_lineage = function()
    assert_fresh_merge_wait_does_not_extend_absolute_cap("merge-ready", lineages.review_loop)
  end,

  test_merge_ready_fresh_merge_gate_wait_within_absolute_cap_defers = function()
    assert_fresh_merge_wait_defers_within_absolute_cap("merge-ready")
  end,

  test_merging_fresh_merge_gate_wait_within_absolute_cap_defers = function()
    assert_fresh_merge_wait_defers_within_absolute_cap("merging")
  end,

  test_merge_ready_stale_merge_gate_wait_under_budget_falls_back_to_state_age = function()
    assert_stale_merge_wait_falls_back_to_under_budget_state_age("merge-ready")
  end,

  test_merging_stale_merge_gate_wait_under_budget_falls_back_to_state_age = function()
    assert_stale_merge_wait_falls_back_to_under_budget_state_age("merging")
  end,

  test_merge_ready_stale_or_missing_merge_gate_wait_escalates_row_budget = function()
    assert_stale_or_missing_merge_wait_escalates("merge-ready", merge_gate_wait_comment(version, "2026-06-03T00:00:00Z"))
    assert_stale_or_missing_merge_wait_escalates("merge-ready", nil)
  end,

  test_merging_stale_or_missing_merge_gate_wait_escalates_row_budget = function()
    assert_stale_or_missing_merge_wait_escalates("merging", merge_gate_wait_comment(version, "2026-06-03T00:00:00Z"))
    assert_stale_or_missing_merge_wait_escalates("merging", nil)
  end,

  test_timeout_reconcile_why_reports_merge_gate_wait_age = function()
    local row = core.restart_transition_row("merge-ready")
    local timeout_version = version .. "/timeout/merge-ready/3"
    local source_ref = core.pr_source_ref(repo, 7)
    local wait_age_minutes = 391
    local payload = core.build_devloop_timeout_reconcile_payload(row, {
      state = "merge-ready",
      version = timeout_version,
    }, proposal_id, source_ref, 3)
    local result = run_timeout_reconcile(payload, {
      state_comment("merge-ready", timeout_version, "2026-06-03T00:00:00Z"),
      timeout_attempt_comment("merge-ready", version, 1, source_ref),
      timeout_attempt_comment("merge-ready", version, 2, source_ref),
      merge_gate_wait_comment(version, os.date("!%Y-%m-%dT%H:%M:%SZ", now() - wait_age_minutes * 60)),
    }, "timeout-reconcile-merge-gate-wait-age")
    t.eq(result.exit_code, 0)
    local comment = h.find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment ~= nil)
    t.is_true(comment.payload.body:find("age_minutes=" .. tostring(wait_age_minutes), 1, true) == nil)
    t.is_true(comment.payload.body:find("reason_class=external-ci-wait-expired", 1, true) ~= nil)
    t.is_true(comment.payload.body:find("reason_class=\"external-ci-wait-expired\"", 1, true) ~= nil)
  end,

  test_timeout_reconcile_why_reports_fix_lineage_merge_gate_wait_age = function()
    local row = core.restart_transition_row("merge-ready")
    local timeout_version = lineages.fix .. "/timeout/merge-ready/3"
    local source_ref = core.pr_source_ref(repo, 7)
    local wait_age_minutes = 391
    local payload = core.build_devloop_timeout_reconcile_payload(row, {
      state = "merge-ready",
      version = timeout_version,
    }, proposal_id, source_ref, 3)
    local result = run_timeout_reconcile(payload, {
      state_comment("merge-ready", timeout_version, "2026-06-03T00:00:00Z"),
      timeout_attempt_comment("merge-ready", lineages.fix, 1, source_ref),
      timeout_attempt_comment("merge-ready", lineages.fix, 2, source_ref),
      merge_gate_wait_comment(lineages.fix, os.date("!%Y-%m-%dT%H:%M:%SZ", now() - wait_age_minutes * 60)),
    }, "timeout-reconcile-fix-lineage-merge-gate-wait-age")
    t.eq(result.exit_code, 0)
    local comment = h.find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment ~= nil)
    t.is_true(comment.payload.body:find("age_minutes=" .. tostring(wait_age_minutes), 1, true) == nil)
    t.is_true(comment.payload.body:find("reason_class=external-ci-wait-expired", 1, true) ~= nil)
  end,

  test_timeout_reconcile_why_reports_review_loop_lineage_merge_gate_wait_age = function()
    local row = core.restart_transition_row("merge-ready")
    local timeout_version = lineages.review_loop .. "/timeout/merge-ready/3"
    local source_ref = core.pr_source_ref(repo, 7)
    local wait_age_minutes = 391
    local payload = core.build_devloop_timeout_reconcile_payload(row, {
      state = "merge-ready",
      version = timeout_version,
    }, proposal_id, source_ref, 3)
    local result = run_timeout_reconcile(payload, {
      state_comment("merge-ready", timeout_version, "2026-06-03T00:00:00Z"),
      timeout_attempt_comment("merge-ready", lineages.review_loop, 1, source_ref),
      timeout_attempt_comment("merge-ready", lineages.review_loop, 2, source_ref),
      merge_gate_wait_comment(lineages.review_loop, os.date("!%Y-%m-%dT%H:%M:%SZ", now() - wait_age_minutes * 60)),
    }, "timeout-reconcile-review-loop-lineage-merge-gate-wait-age")
    t.eq(result.exit_code, 0)
    local comment = h.find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment ~= nil)
    t.is_true(comment.payload.body:find("age_minutes=" .. tostring(wait_age_minutes), 1, true) == nil)
    t.is_true(comment.payload.body:find("reason_class=external-ci-wait-expired", 1, true) ~= nil)
  end,
}
