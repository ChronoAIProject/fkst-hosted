local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function copy_value(value)
  if type(value) ~= "table" then
    return value
  end
  local out = {}
  for key, nested in pairs(value) do
    out[key] = copy_value(nested)
  end
  return out
end

local function copy_rows(rows)
  local copied = {}
  for index, row in ipairs(rows or {}) do
    copied[index] = copy_value(row)
  end
  return copied
end

local function rows_by_state(rows)
  local by_state = {}
  for _, row in ipairs(rows or {}) do
    by_state[row.from_state] = row
  end
  return by_state
end

local function joined_errors(errors)
  return table.concat(errors or {}, "\n")
end

local function contains_error(errors, needle)
  return joined_errors(errors):find(needle, 1, true) ~= nil
end

local function assert_inventory_errors(inventory, state, expected)
  local listed = inventory[state]
  t.eq(type(listed), "table", state)
  local count = 0
  for err, reason in pairs(listed) do
    t.eq(type(reason), "string", err)
    t.is_true(reason ~= "", err)
    t.is_true(expected[err] == true, err)
    count = count + 1
  end
  local expected_count = 0
  for err, _ in pairs(expected) do
    t.is_true(listed[err] ~= nil, err)
    expected_count = expected_count + 1
  end
  t.eq(count, expected_count, state)
end

local function clean_row()
  return {
    from_state = "synthetic-clean",
    terminal = false,
    to_states = { "synthetic-done" },
    driving_queue = "synthetic_queue",
    liveness_class_id = "synthetic.clean",
    responsibility_signature = {
      receiver_kind = "synthetic-worker",
      driving_queue = "synthetic_queue",
      state_kind = "worker",
      liveness_class = "synthetic.clean",
      input_fact_family = "synthetic-input",
      output_postcondition_family = "synthetic-output",
      phase_rank = 0,
      lineage_keys = { "state.version" },
      successors = {
        {
          state = "synthetic-done",
          output_variant = "done",
          postcondition_family = "synthetic-output",
          monotonic = true,
        },
      },
    },
  }
end

local function set_clean_signature(row)
  local signature = clean_row().responsibility_signature
  signature.driving_queue = row.driving_queue
  signature.liveness_class = row.liveness_class_id
  signature.phase_rank = core.stage_rank(row.from_state)
  signature.successors = {}
  for _, next_state in ipairs(row.to_states or {}) do
    table.insert(signature.successors, {
      state = next_state,
      output_variant = next_state,
      postcondition_family = signature.output_postcondition_family,
      monotonic = true,
    })
  end
  row.responsibility_signature = signature
  return signature
end

return {
  test_known_god_states_inventory_is_exact = function()
    local inventory = core.known_god_states()
    assert_inventory_errors(inventory, "ready", {
      ["ready: non-terminal row must declare responsibility_signature"] = true,
    })
    assert_inventory_errors(inventory, "merge-ready", {
      ["merge-ready: non-terminal row must declare responsibility_signature"] = true,
    })
    assert_inventory_errors(inventory, "blocked", {
      ["blocked: non-terminal row must declare responsibility_signature"] = true,
    })
    local count = 0
    for _ in pairs(inventory) do
      count = count + 1
    end
    t.eq(count, 3)
  end,

  test_inventory_ratchet_keeps_main_conformance_green = function()
    t.eq(#core.liveness_contract_errors(), 0)
    local strict = core.strict_restart_responsibility_contract_errors()
    for _, state in ipairs({ "ready", "merge-ready", "blocked" }) do
      t.is_true(core.responsibility_contract_inventory_is_listed_violation(state, strict), state)
    end
    t.eq(core.responsibility_contract_inventory_is_listed_violation("reviewing", strict), false)
    t.eq(core.responsibility_contract_inventory_is_listed_violation("implementing", strict), false)
    t.eq(core.responsibility_contract_inventory_is_listed_violation("fixing", strict), false)
    t.eq(core.responsibility_contract_inventory_is_listed_violation("pr-open", strict), false)
    t.eq(core.responsibility_contract_inventory_is_listed_violation("merging", strict), false)
  end,

  test_clean_single_responsibility_rows_pass_strict_contract = function()
    local by_state = rows_by_state(core.restart_transition_table())
    for _, state in ipairs({ "thinking", "implementing", "impl-failed", "pr-open", "reviewing", "review-meta", "merging", "fixing" }) do
      local errors = core.strict_restart_responsibility_contract_errors({ by_state[state] })
      t.eq(#errors, 0, state .. ": " .. joined_errors(errors))
    end
    t.eq(#core.strict_restart_responsibility_contract_errors({ clean_row() }), 0)
  end,

  test_reviewing_is_clean_review_decision_signature = function()
    local row = rows_by_state(core.restart_transition_table()).reviewing
    local signature = row.responsibility_signature
    t.eq(signature.state_kind, "decision")
    t.eq(signature.receiver_kind, "reviewer")
    t.eq(signature.driving_queue, "devloop_reviewing")
    t.eq(signature.liveness_class, "reviewing.active")
    t.eq(signature.output_postcondition_family, "review_decision_recorded")
    t.eq(signature.decision_type, "ReviewDecision")
    local by_state = {}
    for _, edge in ipairs(signature.successors) do
      by_state[edge.state] = edge
    end
    t.eq(by_state["merge-ready"].output_variant, "approved")
    t.eq(by_state["merge-ready"].decision_type, "ReviewDecision")
    t.eq(by_state["merge-ready"].postcondition_family, "review_decision_recorded")
    t.eq(by_state["fixing"].output_variant, "changes_requested")
    t.eq(by_state["fixing"].decision_type, "ReviewDecision")
    t.eq(by_state["fixing"].postcondition_family, "review_decision_recorded")
    t.eq(by_state["fixing"].bump, true)
    t.eq(by_state["review-meta"].output_variant, "needs_review_meta")
    t.eq(by_state["review-meta"].decision_type, "ReviewDecision")
    t.eq(by_state["review-meta"].postcondition_family, "review_decision_recorded")
    t.eq(by_state.blocked.output_variant, "watchdog_reconcile_terminal")
    t.eq(by_state.blocked.terminal, true)
    t.eq(by_state.blocked.decision_type, nil)
    t.eq(by_state.blocked.postcondition_family, nil)
  end,

  test_terminal_escape_to_non_terminal_state_fails = function()
    local row = {
      from_state = "synthetic-terminal-escape",
      terminal = false,
      to_states = { "synthetic-forward", "blocked" },
      driving_queue = "synthetic_decision",
      liveness_class_id = "synthetic.terminal_escape",
      responsibility_signature = {
        receiver_kind = "synthetic-judge",
        driving_queue = "synthetic_decision",
        state_kind = "decision",
        liveness_class = "synthetic.terminal_escape",
        input_fact_family = "synthetic-input",
        output_postcondition_family = "synthetic-decision-result",
        decision_type = "synthetic-decision-result",
        phase_rank = 10,
        lineage_keys = { "state.version" },
        successors = {
          {
            state = "synthetic-forward",
            output_variant = "forward",
            terminal = true,
            monotonic = true,
          },
          {
            state = "blocked",
            output_variant = "blocked-terminal",
            terminal = true,
            monotonic = true,
          },
        },
      },
    }
    local forward = copy_value(row)
    forward.from_state = "synthetic-forward"
    forward.to_states = { "synthetic-done" }
    forward.driving_queue = "synthetic_forward"
    forward.liveness_class_id = "synthetic.forward"
    forward.responsibility_signature.receiver_kind = "synthetic-worker"
    forward.responsibility_signature.driving_queue = "synthetic_forward"
    forward.responsibility_signature.state_kind = "worker"
    forward.responsibility_signature.liveness_class = "synthetic.forward"
    forward.responsibility_signature.input_fact_family = "synthetic-forward-input"
    forward.responsibility_signature.output_postcondition_family = "synthetic-forward-output"
    forward.responsibility_signature.phase_rank = 20
    forward.responsibility_signature.decision_type = nil
    forward.responsibility_signature.successors = {
      {
        state = "synthetic-done",
        output_variant = "done",
        postcondition_family = "synthetic-forward-output",
        monotonic = true,
      },
    }
    local blocked = copy_value(row)
    blocked.from_state = "blocked"
    blocked.to_states = {}
    blocked.responsibility_signature = nil
    local original_stage_rank = core.stage_rank
    core.stage_rank = function(state)
      if state == "synthetic-terminal-escape" then
        return 10
      end
      if state == "synthetic-forward" then
        return 20
      end
      if state == "synthetic-done" then
        return 30
      end
      return original_stage_rank(state)
    end
    local ok, errors = pcall(core.strict_restart_responsibility_contract_errors, { row, forward, blocked })
    core.stage_rank = original_stage_rank
    if not ok then
      error(errors)
    end
    t.is_true(contains_error(errors, "synthetic-terminal-escape: terminal-escape successor must point to a terminal-class state: synthetic-forward"), joined_errors(errors))
    t.is_true(not contains_error(errors, "synthetic-terminal-escape: terminal-escape successor must point to a terminal-class state: blocked"), joined_errors(errors))
  end,

  test_merge_ready_is_flagged_as_worst_god_state = function()
    local row = rows_by_state(core.restart_transition_table())["merge-ready"]
    t.eq(row.responsibility_signature, nil)
    t.eq(#row.to_states, 4)
    t.is_true(contains_error(core.strict_restart_responsibility_contract_errors({ row }), "merge-ready: non-terminal row must declare responsibility_signature"))
    t.is_true(row.to_states[1] == "reviewing")
    t.is_true(core.stage_rank("reviewing") < core.stage_rank("merge-ready"))
  end,

  test_negative_control_unrelated_families_backward_edge_and_two_receivers_fail = function()
    local row = clean_row()
    row.from_state = "synthetic-bad"
    row.responsibility_signature.receiver_kind = { "worker-a", "worker-b" }
    row.responsibility_signature.successors[1].postcondition_family = "other-family"
    row.responsibility_signature.phase_rank = 20
    row.responsibility_signature.successors[1].state = "synthetic-earlier"
    row.to_states = { "synthetic-earlier" }
    local original_stage_rank = core.stage_rank
    core.stage_rank = function(state)
      if state == "synthetic-bad" then
        return 20
      end
      if state == "synthetic-earlier" then
        return 10
      end
      return original_stage_rank(state)
    end
    local ok, errors = pcall(core.strict_restart_responsibility_contract_errors, { row })
    core.stage_rank = original_stage_rank
    if not ok then
      error(errors)
    end
    t.is_true(contains_error(errors, "synthetic-bad: responsibility_signature.receiver_kind must be exactly one receiver"))
    t.is_true(contains_error(errors, "synthetic-bad: normal successor has unrelated output_postcondition_family: synthetic-earlier"))
    t.is_true(contains_error(errors, "synthetic-bad: backward successor requires generation bump: synthetic-earlier"))
  end,

  test_anti_allowlist_extra_error_on_listed_state_is_not_suppressed = function()
    local rows = copy_rows(core.restart_transition_table())
    local ready = rows_by_state(rows).ready
    ready.responsibility_signature = {
      receiver_kind = { "dependency-gate", "implementation-kickoff" },
      driving_queue = "wrong_queue",
      state_kind = "queue_wait",
      liveness_class = "ready.actionable",
      input_fact_family = "ready-input",
      output_postcondition_family = "ready-output",
      phase_rank = core.stage_rank("ready"),
      lineage_keys = { "state.version" },
      successors = {
        {
          state = "implementing",
          output_variant = "implementation",
          postcondition_family = "ready-output",
          monotonic = true,
        },
      },
    }
    local errors = core.restart_responsibility_inventory_errors(rows)
    t.is_true(contains_error(errors, "ready: responsibility_signature.receiver_kind must be exactly one receiver"))
    t.is_true(contains_error(errors, "ready: responsibility_signature.driving_queue must match row.driving_queue"))
  end,

  test_inventory_ratchet_rejects_stale_entry_after_signature_added = function()
    local rows = copy_rows(core.restart_transition_table())
    local by_state = rows_by_state(rows)
    local ready = by_state.ready
    ready.responsibility_signature = {
      receiver_kind = "dependency-gate",
      driving_queue = "devloop_ready",
      state_kind = "queue_wait",
      liveness_class = "ready.actionable",
      input_fact_family = "dependency-gate",
      output_postcondition_family = "implementation-kickoff",
      phase_rank = core.stage_rank("ready"),
      lineage_keys = { "state.version", "source_ref" },
      successors = {
        {
          state = "implementing",
          output_variant = "dependency-satisfied",
          postcondition_family = "implementation-kickoff",
          monotonic = true,
        },
      },
    }
    local errors = core.restart_responsibility_inventory_errors(rows)
    t.is_true(contains_error(errors, "ready: listed known_god_states entry is stale and must be removed"))
  end,

  test_known_god_state_with_duplicate_signature_still_fails_rule_6 = function()
    local rows = copy_rows(core.restart_transition_table())
    local by_state = rows_by_state(rows)
    local ready_signature = set_clean_signature(by_state.ready)
    local other = clean_row()
    other.from_state = "synthetic-ready-duplicate"
    other.to_states = { "synthetic-ready-done" }
    other.driving_queue = ready_signature.driving_queue
    other.liveness_class_id = ready_signature.liveness_class
    other.responsibility_signature = copy_value(ready_signature)
    other.responsibility_signature.successors = {
      {
        state = "synthetic-ready-done",
        output_variant = "done",
        postcondition_family = ready_signature.output_postcondition_family,
        monotonic = true,
      },
    }
    table.insert(rows, other)
    local errors = core.restart_responsibility_inventory_errors(rows)
    t.is_true(contains_error(errors, "synthetic-ready-duplicate: duplicate responsibility_signature shared with ready"), joined_errors(errors))
  end,

  test_signature_omitting_real_successor_fails_successor_set_check = function()
    local row = clean_row()
    row.from_state = "synthetic-omits-edge"
    row.to_states = { "synthetic-done", "synthetic-also-done" }
    local errors = core.strict_restart_responsibility_contract_errors({ row })
    t.is_true(contains_error(errors, "synthetic-omits-edge: responsibility_signature.successors missing row successor synthetic-also-done"), joined_errors(errors))
  end,

  test_merge_ready_complete_signature_still_fails_backward_edges = function()
    local row = copy_value(rows_by_state(core.restart_transition_table())["merge-ready"])
    row.responsibility_signature = {
      receiver_kind = "merge-gate-worker",
      driving_queue = "devloop_merge_ready",
      state_kind = "gate",
      liveness_class = "merge_ready.actionable",
      input_fact_family = "merge-authorization",
      output_postcondition_family = "merge-gate-result",
      decision_type = "merge-gate-result",
      phase_rank = core.stage_rank("merge-ready"),
      lineage_keys = { "state.version", "reviewed_head_sha", "source_ref" },
      successors = {
        {
          state = "reviewing",
          output_variant = "new-head-review",
          postcondition_family = "merge-gate-result",
          decision_type = "merge-gate-result",
          monotonic = true,
        },
        {
          state = "merging",
          output_variant = "merge-authorized",
          postcondition_family = "merge-gate-result",
          decision_type = "merge-gate-result",
          monotonic = true,
        },
        {
          state = "fixing",
          output_variant = "merge-needs-fix",
          postcondition_family = "merge-gate-result",
          decision_type = "merge-gate-result",
          monotonic = true,
        },
        {
          state = "blocked",
          output_variant = "merge-blocked",
          postcondition_family = "merge-gate-result",
          decision_type = "merge-gate-result",
          terminal = true,
          monotonic = true,
        },
      },
    }
    local errors = core.strict_restart_responsibility_contract_errors({ row })
    t.is_true(contains_error(errors, "merge-ready: backward successor requires generation bump: reviewing"), joined_errors(errors))
  end,

  test_legit_forward_decision_row_passes = function()
    local row = {
      from_state = "synthetic-decision",
      terminal = false,
      to_states = { "synthetic-forward-a", "synthetic-forward-b" },
      driving_queue = "synthetic_decision",
      liveness_class_id = "synthetic.decision",
      responsibility_signature = {
        receiver_kind = "synthetic-judge",
        driving_queue = "synthetic_decision",
        state_kind = "decision",
        liveness_class = "synthetic.decision",
        input_fact_family = "synthetic-input",
        output_postcondition_family = "synthetic-decision-result",
        decision_type = "synthetic-decision-result",
        phase_rank = 10,
        lineage_keys = { "state.version" },
        successors = {
          {
            state = "synthetic-forward-a",
            output_variant = "a",
            postcondition_family = "synthetic-decision-result",
            decision_type = "synthetic-decision-result",
            monotonic = true,
          },
          {
            state = "synthetic-forward-b",
            output_variant = "b",
            postcondition_family = "synthetic-decision-result",
            decision_type = "synthetic-decision-result",
            monotonic = true,
          },
        },
      },
    }
    local original_stage_rank = core.stage_rank
    core.stage_rank = function(state)
      if state == "synthetic-decision" then
        return 10
      end
      if state == "synthetic-forward-a" or state == "synthetic-forward-b" then
        return 20
      end
      return original_stage_rank(state)
    end
    local ok, errors = pcall(core.strict_restart_responsibility_contract_errors, { row })
    core.stage_rank = original_stage_rank
    if not ok then
      error(errors)
    end
    t.eq(#errors, 0, joined_errors(errors))
  end,
}
