local contract_time = require("contract.time")
local transition_version = require("contract.transition_version")
local entity_lib = require("devloop.entity")
local replay_fields = require("devloop.replay_fields")
local m_rae = require("devloop.restart_actionable_epoch")
local ci_repair_attempts = require("core.ci_repair_attempts")
local ci_repair_retry = require("core.ci_repair_retry")
local config = require("devloop.config")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core

local repo = "owner/repo"

local function with_codex_runs(running, fn)
  local original = fkst.codex_runs
  fkst.codex_runs = function()
    return { running = running or {}, recent = {} }
  end
  local ok, err = pcall(fn)
  fkst.codex_runs = original
  if not ok then error(err) end
end

local function hold_fixture(completed_at)
  local event = h.fixing({
    repair_input = "ci-failure",
    ci_failure_key = "head:def456/checks:digest-0000000101",
  })
  local comments = {
    {
      body = ci_repair_attempts.comment_request(
        repo,
        event,
        "no-fix",
        "No repaired revision was published."
      ).body,
      author_login = "fkst-test-bot",
      created_at = completed_at,
    },
  }
  local state = {
    state = "fixing",
    version = event.version,
    proposal_id = event.proposal_id,
    marker_created_at = "2026-06-03T01:00:00Z",
  }
  local row = replay_fields.restart_transition_row(core.restart_transition_table(), "fixing")
  local facts = {
    proposal_id = event.proposal_id,
    source_ref = entity_lib.pr_source_ref(repo, event.pr_number),
    work_unit_key = event.work_unit_key,
    current_pr = {
      comments = comments,
      head_sha = event.reviewed_head_sha,
    },
    link = { pr_number = event.pr_number },
    snapshot = { comments = comments },
  }
  local delay_seconds = core.version_fix_round(state.version)
    * config.liveness_poll_cadence_seconds()
  local lineage_seconds = contract_time.iso_timestamp_epoch_seconds(
    transition_version.updated_at(state.version)
  )
  return state, row, facts, lineage_seconds + delay_seconds, delay_seconds
end

return {
  test_legitimate_completion_after_fix_watchdog_is_not_poisoned = function()
    local completed_at = "2026-06-03T03:30:00Z"
    local state, row, facts, _, delay_seconds = hold_fixture(completed_at)
    local completed_seconds = contract_time.iso_timestamp_epoch_seconds(completed_at)
    local state_entry_seconds = math.max(
      contract_time.iso_timestamp_epoch_seconds(state.marker_created_at),
      contract_time.iso_timestamp_epoch_seconds(transition_version.updated_at(state.version))
    )
    t.is_true(completed_seconds > state_entry_seconds + math.floor(row.watchdog.budget_ms / 1000))
    local due_seconds = completed_seconds + delay_seconds
    local hold = ci_repair_retry.resolve_liveness_hold(row, state, facts, due_seconds)
    t.eq(hold.status, "released")
    t.eq(hold.completed_seconds, completed_seconds)
    t.eq(hold.due_ms, due_seconds * 1000)
    t.eq(hold.poisoned, nil)
  end,

  test_invalid_durable_contract_cannot_fall_through_to_stale_live_run = function()
    local state, row, facts, due_seconds = hold_fixture("not-a-timestamp")
    local expected_dedup_key
    with_codex_runs({}, function()
      expected_dedup_key = core.restart_row_liveness_signal(
        row,
        state,
        facts,
        due_seconds
      ).expected_dedup_key
    end)
    local ok, hold = pcall(
      ci_repair_retry.resolve_liveness_hold,
      row,
      state,
      facts,
      due_seconds
    )
    t.eq(ok, true)
    t.eq(hold.status, "contract_invalid")
    with_codex_runs({
      {
        run_id = "stale-invalid-contract-fixing-run",
        role = "fix",
        proposal_id = state.proposal_id,
        dedup_key = expected_dedup_key,
        status = "running",
        lease_expires_at_ms = (due_seconds + math.floor(row.watchdog.budget_ms / 1000)) * 1000,
      },
    }, function()
      local eval = m_rae.actionable_epoch_resolve(core, row, state, facts, due_seconds)
      t.eq(eval.status, "contract_invalid")
      t.eq(eval.signal, nil)
      local receiver = core.restart_row_receiver_liveness(row, state, facts, due_seconds)
      t.eq(receiver.action, "stuck")
    end)
  end,
}
