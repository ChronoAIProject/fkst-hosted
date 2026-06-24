local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local ready = h.ready

local repo = "owner/repo"

local function state_for(event, version)
  return {
    state = "implementing",
    version = version or event.dedup_key,
    proposal_id = event.proposal_id,
    marker_created_at = "2026-06-03T00:00:00Z",
  }
end

local function facts_for(event, comments, now_seconds)
  return {
    proposal_id = event.proposal_id,
    source_ref = event.source_ref,
    current = {
      comments = comments or {},
      labels = { "fkst-dev:enabled", "fkst-dev:implementing" },
    },
    fresh_current_state = state_for(event),
    now_seconds = now_seconds or core.iso_timestamp_epoch_seconds("2026-06-03T03:00:00Z"),
  }
end

local function with_codex_run_status(status, fn)
  local original = fkst.codex_runs
  fkst.codex_runs = function()
    return status or { running = {}, recent = {} }
  end
  local ok, err = pcall(fn)
  fkst.codex_runs = original
  if not ok then
    error(err)
  end
end

local function with_codex_runs(running, fn)
  return with_codex_run_status({ running = running or {}, recent = {} }, fn)
end

local function capture_raises(fn)
  local raised = {}
  local original = core.log_raise
  core.log_raise = function(_, _, queue, payload)
    table.insert(raised, { queue = queue, payload = payload })
  end
  local ok, err = pcall(fn)
  core.log_raise = original
  if not ok then
    error(err)
  end
  return raised
end

local function captured_raise(raised, queue, predicate)
  for _, item in ipairs(raised or {}) do
    if item.queue == queue and (predicate == nil or predicate(item.payload, item)) then
      return item
    end
  end
  return nil
end

local function trusted_comment(body)
  return {
    body = body,
    author_login = "fkst-test-bot",
    created_at = "2026-06-03T00:00:00Z",
  }
end

local function assert_no_timeout_effects(raised)
  t.eq(captured_raise(raised, "devloop_ready"), nil)
  t.eq(captured_raise(raised, "devloop_timeout_reconcile"), nil)
  t.eq(captured_raise(raised, "github-proxy.github_issue_comment_request", function(payload)
    return tostring(payload.body or ""):find("fkst:github-devloop:timeout-attempt", 1, true) ~= nil
  end), nil)
end

local function run_timeout(row, state, facts)
  return capture_raises(function()
    local handled = core.maybe_timeout_redrive_from_table("liveness_scan", {
      repo = repo,
      number = 42,
      source_ref = core.issue_source_ref(repo, 42),
    }, state, row, facts)
    t.eq(handled, true)
  end)
end

return {
  test_implement_live_codex_run_defers_without_attempt_marker = function()
    local event = ready()
    local row = core.restart_transition_row("implementing")
    local state = state_for(event)
    local comments = {
      core.state_marker(event.proposal_id, "implementing", event.dedup_key),
    }
    local facts = facts_for(event, comments)
    with_codex_runs({
      {
        run_id = "implement-live",
        role = "implement",
        proposal_id = event.proposal_id,
        dedup_key = event.dedup_key,
        status = "running",
        started_at = "2026-06-03T02:30:00Z",
        timeout_seconds = 3600,
      },
    }, function()
      local receiver = core.restart_row_receiver_liveness(row, state, facts, facts.now_seconds)
      t.eq(receiver.action, "defer")
      t.eq(receiver.signal.family, "codex_run:v1")
      assert_no_timeout_effects(run_timeout(row, state, facts))
    end)
  end,

  test_implement_live_codex_run_within_deadline_defers = function()
    local event = ready()
    local row = core.restart_transition_row("implementing")
    local state = state_for(event)
    local facts = facts_for(event, {
      core.state_marker(event.proposal_id, "implementing", event.dedup_key),
    }, core.iso_timestamp_epoch_seconds("2026-06-03T00:59:00Z"))
    with_codex_runs({
      {
        run_id = "implement-live-within-deadline",
        role = "implement",
        proposal_id = event.proposal_id,
        dedup_key = event.dedup_key,
        status = "running",
        started_at = "2026-06-03T00:00:00Z",
        timeout_seconds = 3600,
      },
    }, function()
      local signal = core.restart_row_liveness_signal(row, state, facts, facts.now_seconds)
      t.eq(signal.live, true)
      t.eq(signal.reason, "codex-run-running")
      t.eq(signal.deadline_source, "started_at_plus_timeout_seconds")
      local receiver = core.restart_row_receiver_liveness(row, state, facts, facts.now_seconds)
      t.eq(receiver.action, "defer")
      assert_no_timeout_effects(run_timeout(row, state, facts))
    end)
  end,

  test_implement_hung_codex_run_past_deadline_terminates_after_budget = function()
    local event = ready()
    local row = core.restart_transition_row("implementing")
    local timeout_version = event.dedup_key .. "/timeout/implementing/2"
    local state = state_for(event, timeout_version)
    local facts = facts_for(event, {
      core.state_marker(event.proposal_id, "implementing", timeout_version),
    }, core.iso_timestamp_epoch_seconds("2026-06-03T03:00:00Z"))
    with_codex_runs({
      {
        run_id = "implement-live-past-deadline",
        role = "implement",
        proposal_id = event.proposal_id,
        dedup_key = event.dedup_key,
        status = "running",
        started_at = "2026-06-03T00:00:00Z",
        timeout_seconds = 3600,
      },
    }, function()
      local eval = core.actionable_epoch_resolve(row, state, facts, facts.now_seconds)
      t.eq(eval.status, "actionable")
      t.eq(eval.signal.reason, "codex-run-deadline-expired")
      table.insert(facts.current.comments, trusted_comment(core.timeout_attempt_v2_marker(
        event.proposal_id,
        row.from_state,
        row.liveness_class_id,
        eval.generation_key,
        1,
        event.source_ref
      )))
      table.insert(facts.current.comments, trusted_comment(core.timeout_attempt_v2_marker(
        event.proposal_id,
        row.from_state,
        row.liveness_class_id,
        eval.generation_key,
        2,
        event.source_ref
      )))
      local due, age = core.liveness_timeout_due_with_facts(row, state, facts, facts.now_seconds)
      t.eq(due, true)
      t.eq(age, 180)
      local raised = run_timeout(row, state, facts)
      t.eq(captured_raise(raised, "devloop_ready"), nil)
      local reconcile = captured_raise(raised, "devloop_timeout_reconcile")
      t.is_true(reconcile ~= nil)
      t.eq(reconcile.payload.state, "implementing")
      t.eq(reconcile.payload.round, 3)
    end)
  end,

  test_implement_recent_codex_run_within_handoff_window_defers = function()
    local event = ready()
    local row = core.restart_transition_row("implementing")
    local state = state_for(event)
    local facts = facts_for(event, {
      core.state_marker(event.proposal_id, "implementing", event.dedup_key),
    }, core.iso_timestamp_epoch_seconds("2026-06-03T00:59:00Z"))
    with_codex_run_status({
      running = {},
      recent = {
        {
          run_id = "implement-recent-within-deadline",
          role = "implement",
          proposal_id = event.proposal_id,
          dedup_key = event.dedup_key,
          status = "done",
          started_at = "2026-06-03T00:00:00Z",
          timeout_seconds = 3600,
          exit_code = 0,
        },
      },
    }, function()
      local signal = core.restart_row_liveness_signal(row, state, facts, facts.now_seconds)
      t.eq(signal.live, true)
      t.eq(signal.reason, "codex-run-recent-handoff")
      t.eq(signal.collection, "recent")
      local receiver = core.restart_row_receiver_liveness(row, state, facts, facts.now_seconds)
      t.eq(receiver.action, "defer")
      assert_no_timeout_effects(run_timeout(row, state, facts))
    end)
  end,

  test_implement_codex_runs_unavailable_falls_back_to_marker_budget_terminate = function()
    local event = ready()
    local row = core.restart_transition_row("implementing")
    local timeout_version = event.dedup_key .. "/timeout/implementing/2"
    local state = state_for(event, timeout_version)
    local facts = facts_for(event, {
      core.state_marker(event.proposal_id, "implementing", timeout_version),
    }, core.iso_timestamp_epoch_seconds("2026-06-03T03:00:00Z"))
    local original = fkst.codex_runs
    fkst.codex_runs = function()
      error("synthetic codex_runs failure")
    end
    local ok, err = pcall(function()
      local eval = core.actionable_epoch_resolve(row, state, facts, facts.now_seconds)
      t.eq(eval.status, "actionable")
      t.eq(eval.signal.reason, "codex-runs-unavailable")
      t.eq(eval.codex_runs_fallback, true)
      local due, age = core.liveness_timeout_due_with_facts(row, state, facts, facts.now_seconds)
      t.eq(due, true)
      t.eq(age, 180)
      local receiver = core.restart_row_receiver_liveness(row, state, facts, facts.now_seconds)
      t.eq(receiver.action, "stuck")
      local raised = run_timeout(row, state, facts)
      t.eq(captured_raise(raised, "devloop_ready"), nil)
      local reconcile = captured_raise(raised, "devloop_timeout_reconcile")
      t.is_true(reconcile ~= nil)
      t.eq(reconcile.payload.state, "implementing")
      t.eq(reconcile.payload.round, 3)
    end)
    fkst.codex_runs = original
    if not ok then
      error(err)
    end
  end,

  test_implement_running_codex_run_without_deadline_falls_back_to_marker_budget_terminate = function()
    local event = ready()
    local row = core.restart_transition_row("implementing")
    local timeout_version = event.dedup_key .. "/timeout/implementing/2"
    local state = state_for(event, timeout_version)
    local facts = facts_for(event, {
      core.state_marker(event.proposal_id, "implementing", timeout_version),
    }, core.iso_timestamp_epoch_seconds("2026-06-03T03:00:00Z"))
    with_codex_runs({
      {
        run_id = "implement-running-missing-deadline",
        role = "implement",
        proposal_id = event.proposal_id,
        dedup_key = event.dedup_key,
        status = "running",
      },
    }, function()
      local eval = core.actionable_epoch_resolve(row, state, facts, facts.now_seconds)
      t.eq(eval.status, "actionable")
      t.eq(eval.signal.reason, "codex-run-deadline-unavailable")
      t.eq(eval.indeterminate, true)
      local due, age = core.liveness_timeout_due_with_facts(row, state, facts, facts.now_seconds)
      t.eq(due, true)
      t.eq(age, 180)
      local receiver = core.restart_row_receiver_liveness(row, state, facts, facts.now_seconds)
      t.eq(receiver.action, "stuck")
      local raised = run_timeout(row, state, facts)
      t.eq(captured_raise(raised, "devloop_ready"), nil)
      local reconcile = captured_raise(raised, "devloop_timeout_reconcile")
      t.is_true(reconcile ~= nil)
      t.eq(reconcile.payload.state, "implementing")
      t.eq(reconcile.payload.round, 3)
    end)
  end,

  test_implement_codex_run_match_preserves_reimplement_suffix = function()
    local event = ready()
    local retry_version = core.implementation_attempt_version(event.dedup_key, 2)
    local row = core.restart_transition_row("implementing")
    local state = state_for(event, retry_version)
    local facts = facts_for(event, {
      core.state_marker(event.proposal_id, "implementing", retry_version),
    })
    with_codex_runs({
      {
        run_id = "base-only-wrong",
        role = "implement",
        proposal_id = event.proposal_id,
        dedup_key = event.dedup_key,
        status = "running",
      },
    }, function()
      local signal = core.restart_row_liveness_signal(row, state, facts, facts.now_seconds)
      t.eq(signal.live, false)
      t.eq(signal.expected_dedup_key, retry_version)
    end)
  end,
}
