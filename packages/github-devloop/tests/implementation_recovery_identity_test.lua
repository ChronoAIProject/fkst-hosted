local h = require("tests.devloop_helpers")
local conv_attempts = require("devloop.convergence.attempts")
local contract_time = require("contract.time")
local entity_lib = require("devloop.entity")

local core = h.core
local t = h.t

local function restart_row(state)
  for _, row in ipairs(core.restart_transition_table()) do
    if row.from_state == state then
      return row
    end
  end
  return nil
end

local function trusted_comment(body, created_at)
  return {
    body = body,
    author_login = "fkst-test-bot",
    created_at = created_at,
  }
end

local function with_indeterminate_codex_runs(fn)
  local original = fkst.codex_runs
  fkst.codex_runs = function()
    error("synthetic codex_runs failure")
  end
  local ok, err = pcall(fn)
  fkst.codex_runs = original
  if not ok then
    error(err)
  end
end

return {
  test_indeterminate_implement_liveness_preserves_timeout_generation_and_attempts = function()
    local event = h.ready()
    local row = restart_row("implementing")
    local source_ref = entity_lib.issue_source_ref("owner/repo", 42)
    local entered_at = "2026-06-03T00:00:00Z"
    local state = {
      state = "implementing",
      version = event.dedup_key,
      proposal_id = event.proposal_id,
      marker_created_at = entered_at,
    }
    local comments = {
      trusted_comment(core.state_marker(event.proposal_id, "implementing", event.dedup_key), entered_at),
      trusted_comment(core.implement_attempt_marker(
        event.proposal_id,
        event.dedup_key,
        1,
        tostring(contract_time.iso_timestamp_epoch_seconds(entered_at)),
        core.implement_exec_ref(event.proposal_id, event.dedup_key)
      ), entered_at),
    }
    local facts = {
      proposal_id = event.proposal_id,
      source_ref = source_ref,
      current = { comments = comments },
      fresh_current_state = state,
    }
    local entered = contract_time.iso_timestamp_epoch_seconds(entered_at)

    with_indeterminate_codex_runs(function()
      local due = core.liveness_timeout_due_with_facts(row, state, facts, entered + ((row.budget.minutes - 1) * 60))
      t.eq(due, false)
      t.eq(facts.actionable_epoch_eval.status, "deferred")
      t.eq(facts.actionable_epoch_eval.reason, "codex run liveness is indeterminate")

      due = core.liveness_timeout_due_with_facts(row, state, facts, entered + ((row.budget.minutes + 1) * 60))
      t.eq(due, true)
      local generation = facts.actionable_epoch_eval.generation_key
      t.is_true(type(generation) == "string" and generation ~= "")

      table.insert(comments, trusted_comment(conv_attempts.timeout_attempt_v2_marker(
        event.proposal_id,
        row.from_state,
        row.liveness_class_id,
        generation,
        1,
        source_ref
      ), "2026-06-03T02:01:00Z"))
      table.insert(comments, trusted_comment(conv_attempts.timeout_attempt_v2_marker(
        event.proposal_id,
        row.from_state,
        row.liveness_class_id,
        generation,
        2,
        source_ref
      ), "2026-06-03T02:02:00Z"))

      local decision = core.liveness_timeout_decision_with_facts(
        row,
        state,
        facts,
        entered + ((row.budget.minutes + 30) * 60)
      )
      t.eq(facts.actionable_epoch_eval.generation_key, generation)
      t.eq(core.liveness_timeout_attempt(row, state, facts), 2)
      t.eq(decision.action, "escalate")
      t.eq(decision.attempt, 3)
    end)
  end,
}
