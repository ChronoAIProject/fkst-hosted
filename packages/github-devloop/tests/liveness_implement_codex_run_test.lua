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
      },
    }, function()
      local receiver = core.restart_row_receiver_liveness(row, state, facts, facts.now_seconds)
      t.eq(receiver.action, "defer")
      t.eq(receiver.signal.family, "codex_run:v1")
      local raised = capture_raises(function()
        local handled = core.maybe_timeout_redrive_from_table("liveness_scan", {
          repo = repo,
          number = 42,
          source_ref = core.issue_source_ref(repo, 42),
        }, state, row, facts)
        t.eq(handled, true)
      end)
      t.eq(captured_raise(raised, "devloop_ready"), nil)
      t.eq(captured_raise(raised, "devloop_timeout_reconcile"), nil)
      t.eq(captured_raise(raised, "github-proxy.github_issue_comment_request", function(payload)
        return tostring(payload.body or ""):find("fkst:github-devloop:timeout-attempt", 1, true) ~= nil
      end), nil)
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
