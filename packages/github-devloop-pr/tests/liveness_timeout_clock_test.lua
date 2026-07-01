local entity_lib = require("devloop.entity")
local devloop_base = require("devloop.base")
local convergence_shared = require("devloop.convergence.shared")
local contract_time = require("contract.time")
local h = require("tests.devloop_helpers")
local conv_rounds = require("devloop.convergence.rounds")
local conv_reconcile = require("devloop.convergence.reconcile")
local conv_attempts = require("devloop.convergence.attempts")
local m_mgw = require("devloop.merge_gate_wait")
local t = h.t
local core = h.core
local opts = h.opts
local replay_fields = require("devloop.replay_fields")
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local repo = "owner/repo"
local proposal_id = "github-devloop/issue/owner/repo/42"
local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local head_sha = "def456"

local function restart_transition_row(state_name)
  return replay_fields.restart_transition_row(core.restart_transition_table(), state_name)
end

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
  local source_ref = entity_lib.issue_source_ref(repo, 42)
  return trusted_comment(conv_rounds.converge_round_marker(core,
    proposal_id,
    version,
    convergence_shared.source_ref_digest(source_ref),
    1,
    "consensus:" .. proposal_id .. "/loop/1",
    "Still thinking",
    { { angle = "minimal", verdict = "continue", digest = "recent" } }
  ), created_at)
end

local function merge_gate_wait_comment(state_version, created_at)
  return trusted_comment(m_mgw.merge_gate_wait_marker(core, proposal_id, 7, state_version, head_sha, "ci-wait", "CI_WAIT"), created_at)
end

local function timeout_attempt_comment(state_name, state_version, round, source_ref)
  return trusted_comment(conv_attempts.timeout_attempt_marker(core, proposal_id, state_version, state_name, round, source_ref), "2026-06-03T00:00:00Z")
end

local function timeout_attempt_v2_comment(row, generation_key, round, source_ref)
  return trusted_comment(conv_attempts.timeout_attempt_v2_marker(core, proposal_id, row.from_state, row.liveness_class_id, generation_key, round, source_ref), "2026-06-03T00:00:00Z")
end

local function implementing_attempt_comment(state_version, started_at, created_at, attempt, exec_ref)
  return trusted_comment(core.implement_attempt_marker(
    proposal_id,
    state_version,
    attempt or 1,
    started_at,
    exec_ref
  ), created_at or os.date("!%Y-%m-%dT%H:%M:%SZ", started_at))
end

local function implement_codex_run(state_version)
  return {
    run_id = "test-implement-run",
    role = "implement",
    proposal_id = proposal_id,
    dedup_key = state_version,
    status = "running",
    started_at = "2026-06-03T00:00:00Z",
    started_at_ms = 1780444800000,
    timeout_seconds = 3600,
  }
end

local function with_codex_runs(running, fn)
  local original = fkst.codex_runs
  fkst.codex_runs = function()
    return { running = running or {}, recent = {} }
  end
  local ok, err = pcall(fn)
  fkst.codex_runs = original
  if not ok then
    error(err)
  end
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
    source_ref = entity_lib.pr_source_ref(repo, 7),
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
  local row = restart_transition_row(state_name)
  local now_seconds = contract_time.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z")
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
      source_ref = entity_lib.issue_source_ref(repo, 42),
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
  local row = restart_transition_row(state_name)
  local now_seconds = contract_time.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z")
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
      source_ref = entity_lib.issue_source_ref(repo, 42),
    }, recent_merge_state(state_name, version), row, merge_timeout_facts({ wait }, now_seconds))
    t.eq(applied, true)
  end)
  t.eq(#raised, 0)
end

local function assert_stale_or_missing_merge_wait_escalates(state_name, wait_comment, lineage_version)
  local row = restart_transition_row(state_name)
  local now_seconds = contract_time.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z")
  local wait_lineage = lineage_version or version
  local timeout_version = wait_lineage .. "/timeout/" .. state_name .. "/3"
  local raised = capture_raises(function()
    local applied = core.maybe_timeout_redrive_from_table("liveness_scan", {
      repo = repo,
      number = 42,
      source_ref = entity_lib.issue_source_ref(repo, 42),
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
  local row = restart_transition_row(state_name)
  local now_seconds = contract_time.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z")
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
      source_ref = entity_lib.issue_source_ref(repo, 42),
    }, recent_merge_state(state_name, version), row, merge_timeout_facts({ stale_wait }, now_seconds))
    t.eq(applied, false)
  end)
  t.eq(#raised, 0)
end

local function run_timeout_reconcile(payload, comments, name, now_seconds)
  local source_repo, source_pr = devloop_base.parse_pr_source_ref(payload and payload.source_ref)
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
  if now_seconds == nil then
    return t.run_department("departments/reconcile/main.lua", {
      queue = "devloop_timeout_reconcile",
      payload = payload,
    }, opts(name or "liveness-timeout-clock"))
  end

  local raised = {}
  local original_raise = raise
  local original_now = now
  raise = function(queue, raised_payload)
    table.insert(raised, { queue = queue, payload = raised_payload })
  end
  now = function()
    return now_seconds
  end
  local ok, err = pcall(function()
    local department = require("departments.reconcile.main")
    department.pipeline({
      queue = "devloop_timeout_reconcile",
      payload = payload,
    })
  end)
  now = original_now
  raise = original_raise
  if not ok then
    return {
      exit_code = 1,
      error = tostring(err),
      raises = raised,
    }
  end
  return {
    exit_code = 0,
    raises = raised,
  }
end

local function timestamp_minutes_before(now_seconds, age_minutes)
  return os.date("!%Y-%m-%dT%H:%M:%SZ", now_seconds - age_minutes * 60)
end

local function timeout_reconcile_age_clock()
  return contract_time.iso_timestamp_epoch_seconds("2026-06-03T06:31:00Z")
end

return {
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
    local row = restart_transition_row("merge-ready")
    local timeout_version = version .. "/timeout/merge-ready/3"
    local source_ref = entity_lib.pr_source_ref(repo, 7)
    local wait_age_minutes = 391
    local now_seconds = timeout_reconcile_age_clock()
    local payload = conv_reconcile.build_devloop_timeout_reconcile_payload(core, row, {
      state = "merge-ready",
      version = timeout_version,
    }, proposal_id, source_ref, 3)
    local result = run_timeout_reconcile(payload, {
      state_comment("merge-ready", timeout_version, "2026-06-03T00:00:00Z"),
      timeout_attempt_comment("merge-ready", version, 1, source_ref),
      timeout_attempt_comment("merge-ready", version, 2, source_ref),
      merge_gate_wait_comment(version, timestamp_minutes_before(now_seconds, wait_age_minutes)),
    }, "timeout-reconcile-merge-gate-wait-age", now_seconds)
    t.eq(result.exit_code, 0)
    local comment = h.find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment ~= nil)
    t.is_true(comment.payload.body:find("age_minutes=" .. tostring(wait_age_minutes), 1, true) ~= nil)
    t.is_true(comment.payload.body:find("reason_class=external-ci-wait-expired", 1, true) ~= nil)
    t.is_true(comment.payload.body:find("reason_class=\"external-ci-wait-expired\"", 1, true) ~= nil)
  end,

  test_timeout_reconcile_why_reports_fix_lineage_merge_gate_wait_age = function()
    local row = restart_transition_row("merge-ready")
    local timeout_version = lineages.fix .. "/timeout/merge-ready/3"
    local source_ref = entity_lib.pr_source_ref(repo, 7)
    local wait_age_minutes = 391
    local now_seconds = timeout_reconcile_age_clock()
    local payload = conv_reconcile.build_devloop_timeout_reconcile_payload(core, row, {
      state = "merge-ready",
      version = timeout_version,
    }, proposal_id, source_ref, 3)
    local result = run_timeout_reconcile(payload, {
      state_comment("merge-ready", timeout_version, "2026-06-03T00:00:00Z"),
      timeout_attempt_comment("merge-ready", lineages.fix, 1, source_ref),
      timeout_attempt_comment("merge-ready", lineages.fix, 2, source_ref),
      merge_gate_wait_comment(lineages.fix, timestamp_minutes_before(now_seconds, wait_age_minutes)),
    }, "timeout-reconcile-fix-lineage-merge-gate-wait-age", now_seconds)
    t.eq(result.exit_code, 0)
    local comment = h.find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment ~= nil)
    t.is_true(comment.payload.body:find("age_minutes=" .. tostring(wait_age_minutes), 1, true) ~= nil)
    t.is_true(comment.payload.body:find("reason_class=external-ci-wait-expired", 1, true) ~= nil)
  end,

  test_timeout_reconcile_why_reports_review_loop_lineage_merge_gate_wait_age = function()
    local row = restart_transition_row("merge-ready")
    local timeout_version = lineages.review_loop .. "/timeout/merge-ready/3"
    local source_ref = entity_lib.pr_source_ref(repo, 7)
    local wait_age_minutes = 391
    local now_seconds = timeout_reconcile_age_clock()
    local payload = conv_reconcile.build_devloop_timeout_reconcile_payload(core, row, {
      state = "merge-ready",
      version = timeout_version,
    }, proposal_id, source_ref, 3)
    local result = run_timeout_reconcile(payload, {
      state_comment("merge-ready", timeout_version, "2026-06-03T00:00:00Z"),
      timeout_attempt_comment("merge-ready", lineages.review_loop, 1, source_ref),
      timeout_attempt_comment("merge-ready", lineages.review_loop, 2, source_ref),
      merge_gate_wait_comment(lineages.review_loop, timestamp_minutes_before(now_seconds, wait_age_minutes)),
    }, "timeout-reconcile-review-loop-lineage-merge-gate-wait-age", now_seconds)
    t.eq(result.exit_code, 0)
    local comment = h.find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment ~= nil)
    t.is_true(comment.payload.body:find("age_minutes=" .. tostring(wait_age_minutes), 1, true) ~= nil)
    t.is_true(comment.payload.body:find("reason_class=external-ci-wait-expired", 1, true) ~= nil)
  end,
}
