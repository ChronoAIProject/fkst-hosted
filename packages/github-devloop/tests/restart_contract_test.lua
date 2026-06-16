local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function has_value(values, expected)
  for _, value in ipairs(values or {}) do
    if value == expected then
      return true
    end
  end
  return false
end

local function copy_rows(rows)
  local copied = {}
  local function copy_value(value)
    if type(value) ~= "table" then
      return value
    end
    local nested = {}
    for nested_key, nested_value in pairs(value) do
      nested[nested_key] = copy_value(nested_value)
    end
    return nested
  end
  for index, row in ipairs(rows or {}) do
    local next_row = {}
    for key, value in pairs(row) do
      next_row[key] = copy_value(value)
    end
    copied[index] = next_row
  end
  return copied
end

local function parse_marker_builders(paths)
  local families = {}
  for _, path in ipairs(paths) do
    local text = file.read(path)
    for family in text:gmatch("fkst:github%-devloop:([%w%-]+):v1") do
      families[family] = families[family] or {}
    end
    for family, attrs in pairs(families) do
      local family_pattern = "fkst:github%-devloop:" .. family:gsub("%-", "%%-") .. ":v1"
      local start_pos = text:find(family_pattern)
      if start_pos ~= nil then
        local function_pos = text:sub(1, start_pos):match("^.*()\nfunction M%.[^\n]+")
        local next_function = text:find("\nfunction M%.", start_pos + 1)
        local block = text:sub(function_pos or start_pos, next_function or #text)
        for attr in block:gmatch('" ([%w_]+)="') do
          attrs[attr] = true
        end
        for attr in block:gmatch('([%w_]+)="') do
          attrs[attr] = true
        end
      end
    end
  end
  return families
end

local function marker_builder_paths()
  return {
    "packages/github-devloop/core/state.lua",
    "packages/github-devloop/core/markers.lua",
    "packages/github-devloop/core/impl_failure.lua",
    "packages/github-devloop/core/convergence.lua",
    "packages/github-devloop/core/dependencies.lua",
    "packages/github-devloop/core/decompose.lua",
    "packages/github-devloop/core/implement_attempt.lua",
  }
end

local function table_by_state()
  local by_state = {}
  for _, row in ipairs(core.restart_transition_table()) do
    by_state[row.from_state] = row
  end
  return by_state
end

local function rows_by_state(rows)
  local by_state = {}
  for _, row in ipairs(rows or {}) do
    by_state[row.from_state] = row
  end
  return by_state
end

local function allowed_extra_transition(state, next_state)
  return (state == "reviewing" and next_state == "blocked")
    or (state == "pr-open" and next_state == "fixing")
    or (state == "impl-failed" and next_state == "implementing")
end

local function capture_raises(fn)
  local raised = {}
  local original_log_raise = core.log_raise
  core.log_raise = function(_, _, queue, payload)
    table.insert(raised, { queue = queue, payload = payload })
  end
  local ok, err = pcall(fn, raised)
  core.log_raise = original_log_raise
  if not ok then
    error(err)
  end
  return raised
end

return {
  test_persistence_class_is_declared = function()
    t.eq(core.persistence_class(), "saga")
  end,

  test_executable_restart_table_covers_non_terminal_states = function()
    local expected = { "thinking", "ready", "implementing", "impl-failed", "pr-open", "reviewing", "merge-ready", "merging", "fixing", "review-meta", "blocked", "merged" }
    local by_state = table_by_state()
    t.eq(#core.liveness_contract_errors(), 0)
    for _, state in ipairs(expected) do
      local row = by_state[state]
      t.is_true(row ~= nil)
      t.eq(row.from_state, state)
      t.is_true(type(row.to_states) == "table")
      t.is_true(type(row.terminal) == "boolean")
      if row.terminal == false then
        t.is_true(type(row.driving_queue) == "string" and row.driving_queue ~= "")
        t.is_true(type(row.output_obligation) == "table")
        t.is_true(type(row.budget) == "table")
        t.is_true(type(row.budget.receiver_max_work_justification) == "string")
        t.is_true(row.budget.receiver_max_work_justification ~= "")
        t.is_true(type(row.liveness_contract) == "table")
        t.is_true(type(row.on_timeout) == "table")
        t.is_true(type(row.payload_builder) == "function")
        t.is_true(type(row.dedup_shape) == "string" and row.dedup_shape ~= "")
        t.is_true(type(row.required_facts) == "table" and #row.required_facts > 0)
        t.is_true(type(row.payload_fields) == "table")
        t.is_true(type(row.version_identity) == "string" and row.version_identity ~= "")
        t.is_true(type(row.effects) == "table")
        t.is_true(tonumber(row.effects.intent_count) ~= nil)
        t.is_true(type(row.effects.kinds) == "table")
        t.eq(#row.effects.kinds, row.effects.intent_count)
        t.is_true(type(row.effects.completeness) == "string" and row.effects.completeness ~= "")
      end
    end
    t.eq(#core.restart_transition_table(), #expected)
  end,

  test_liveness_contract_declares_terminal_taxonomy_and_backstop = function()
    local errors = core.liveness_contract_errors()
    t.eq(#errors, 0)
    local terminals = core.liveness_terminal_states()
    t.eq(#terminals, 1)
    t.eq(terminals[1], "merged")
    local by_state = table_by_state()
    t.eq(by_state["impl-failed"].terminal, false)
    t.eq(by_state["impl-failed"].on_timeout.queue, "devloop_ready")
    t.eq(by_state["impl-failed"].reentry_commands[1], "reready")
    for _, row in ipairs(core.restart_transition_table()) do
      if row.terminal == false then
        t.is_true(row.output_obligation ~= nil)
        t.is_true(tonumber(row.budget.minutes) > 0)
        t.is_true(type(row.budget.receiver_max_work_justification) == "string")
        t.is_true(row.budget.receiver_max_work_justification ~= "")
        t.is_true(type(row.liveness_contract) == "table")
        t.eq(row.on_timeout.action, "redrive")
        t.eq(row.on_timeout.queue, row.driving_queue)
        t.is_true(row.on_timeout.queue ~= "none")
        t.eq(row.on_timeout.on_escalate.action, "force-terminate")
        t.eq(row.on_timeout.on_escalate.terminal_state, "blocked")
        t.eq(row.on_timeout.on_escalate.reason, "state-output-obligation-timeout")
      end
    end
  end,

  test_non_terminal_issue_marker_states_are_liveness_sweep_reachable = function()
    local errors = core.issue_marker_liveness_sweep_contract_errors()
    t.eq(#errors, 0)
    local sweep_states = core.issue_marker_liveness_sweep_states()
    for _, row in ipairs(core.restart_transition_table()) do
      if row.terminal == false then
        t.eq(sweep_states[row.from_state], true)
      else
        t.eq(sweep_states[row.from_state], nil)
      end
    end
    local liveness_scan = file.read("packages/github-devloop/departments/liveness_scan/main.lua")
    local observe_issue = file.read("packages/github-devloop/departments/observe_issue/main.lua")
    t.is_true(liveness_scan:find("core.restart_transition_row", 1, true) ~= nil)
    t.is_true(liveness_scan:find("should_reinject_state", 1, true) ~= nil)
    t.is_true(observe_issue:find("core.restart_row_observable_on", 1, true) ~= nil)
    t.is_true(observe_issue:find("maybe_reconcile_issue_local_orphaned_pr", 1, true) ~= nil)
  end,

  test_issue_marker_liveness_sweep_contract_rejects_missing_non_terminal_state = function()
    local sweep_states = core.issue_marker_liveness_sweep_states()
    sweep_states["pr-open"] = nil
    local errors = core.issue_marker_liveness_sweep_contract_errors(nil, sweep_states)
    t.eq(#errors, 1)
    t.is_true(errors[1]:find("pr-open", 1, true) ~= nil)
    t.is_true(errors[1]:find("liveness sweep", 1, true) ~= nil)
  end,

  test_implementing_restart_row_replays_ready_with_frozen_version_identity = function()
    local row = table_by_state().implementing
    t.eq(row.driving_queue, "devloop_ready")
    t.eq(row.on_timeout.queue, "devloop_ready")
    t.eq(row.kickoff, "devloop_ready")
    t.eq(row.effects.kinds[1], "devloop_ready")
    t.eq(row.payload_builder, core.build_devloop_ready_payload)
    t.eq(row.payload_fields.proposal_id, "marker:state.proposal")
    t.eq(row.payload_fields.dedup_key, "marker:state.version")
    t.is_true(row.version_identity:find("ready_payload_inner_version", 1, true) ~= nil)
  end,

  test_impl_failed_restart_row_replays_ready_with_frozen_version_identity = function()
    local row = table_by_state()["impl-failed"]
    t.eq(row.driving_queue, "devloop_ready")
    t.eq(row.on_timeout.queue, "devloop_ready")
    t.eq(row.kickoff, "devloop_ready")
    t.eq(row.effects.kinds[1], "devloop_ready")
    t.eq(row.payload_builder, core.build_devloop_ready_payload)
    t.eq(row.payload_fields.proposal_id, "marker:state.proposal")
    t.eq(row.payload_fields.dedup_key, "marker:impl-failure.dedup")
    t.is_true(row.version_identity:find("ready_payload_inner_version", 1, true) ~= nil)
  end,

  test_reentry_commands_are_supported_by_operator_parser = function()
    for _, row in ipairs(core.restart_transition_table()) do
      for _, command_name in ipairs(row.reentry_commands or {}) do
        local fact = core.operator_command_fact({
          {
            id = "IC_" .. tostring(row.from_state) .. "_" .. tostring(command_name),
            body = "fkst: " .. tostring(command_name),
            author_login = "fkst-test-bot",
            created_at = "2026-06-04T03:00:00Z",
          },
        }, command_name)
        t.is_true(fact ~= nil, "unsupported reentry command " .. tostring(command_name))
      end
    end
  end,

  test_liveness_contract_rejects_non_terminal_without_output_obligation = function()
    local rows = copy_rows(core.restart_transition_table())
    rows_by_state(rows).ready.output_obligation = nil
    local errors = core.liveness_contract_errors(rows)
    t.eq(#errors, 1)
    t.is_true(errors[1]:find("ready", 1, true) ~= nil)
    t.is_true(errors[1]:find("output_obligation", 1, true) ~= nil)
  end,

  test_liveness_contract_rejects_non_terminal_without_force_termination = function()
    local rows = copy_rows(core.restart_transition_table())
    rows_by_state(rows).ready.on_timeout.on_escalate = nil
    local errors = core.liveness_contract_errors(rows)
    t.eq(#errors, 1)
    t.is_true(errors[1]:find("ready", 1, true) ~= nil)
    t.is_true(errors[1]:find("force-terminate", 1, true) ~= nil)
    t.is_true(errors[1]:find("blocked", 1, true) ~= nil)
  end,

  test_liveness_contract_declares_receiver_liveness_for_every_non_terminal_row = function()
    local by_state = table_by_state()
    local expected = {
      thinking = { mode = "live-defer", family = "converge-round", max_age = 120, budget = 150 },
      ready = { mode = "live-defer", family = "dependency-wait", resolver = "dependency-hold", max_age = 525600, budget = 45 },
      implementing = { mode = "live-defer", family = "implement-attempt", max_age = 120, budget = 45 },
      ["pr-open"] = { mode = "row-budget-bounds-receiver", receiver = 0, budget = 30 },
      reviewing = { mode = "live-defer", family = "review-converge-round", max_age = 120, budget = 150 },
      ["merge-ready"] = { mode = "row-budget-bounds-receiver", receiver = 30, external = 360, budget = 390 },
      merging = { mode = "row-budget-bounds-receiver", receiver = 30, external = 360, budget = 390 },
      fixing = { mode = "row-budget-bounds-receiver", receiver = 60, budget = 120 },
      ["review-meta"] = { mode = "row-budget-bounds-receiver", receiver = 60, budget = 90 },
      ["impl-failed"] = { mode = "row-budget-bounds-receiver", receiver = 0, external = 1410, budget = 1440 },
      blocked = { mode = "row-budget-bounds-receiver", receiver = 0, external = 1410, budget = 1440 },
    }
    for state, spec in pairs(expected) do
      local row = by_state[state]
      t.is_true(row ~= nil, state)
      t.eq(row.terminal, false)
      t.eq(row.budget.minutes, spec.budget)
      t.eq(row.liveness_contract.mode, spec.mode)
      if spec.mode == "live-defer" then
        t.eq(row.liveness_contract.signal.family, spec.family)
        t.eq(row.liveness_contract.signal.resolver, spec.resolver)
        t.eq(row.liveness_contract.signal.producer, spec.family)
        t.eq(row.liveness_contract.signal.max_age_minutes, spec.max_age)
      else
        t.eq(row.liveness_contract.receiver_bound_minutes, spec.receiver)
        t.eq(row.liveness_contract.external_wait_bound_minutes, spec.external)
      end
    end
  end,

  test_liveness_contract_rejects_non_terminal_without_receiver_liveness = function()
    local rows = copy_rows(core.restart_transition_table())
    rows_by_state(rows).ready.liveness_contract = nil
    local errors = core.liveness_contract_errors(rows)
    t.eq(#errors, 1)
    t.is_true(errors[1]:find("ready", 1, true) ~= nil)
    t.is_true(errors[1]:find("liveness_contract", 1, true) ~= nil)
  end,

  test_liveness_contract_rejects_budget_without_receiver_max_work_justification = function()
    local rows = copy_rows(core.restart_transition_table())
    rows_by_state(rows).ready.budget.receiver_max_work_justification = nil
    local errors = core.liveness_contract_errors(rows)
    t.eq(#errors, 1)
    t.is_true(errors[1]:find("ready", 1, true) ~= nil)
    t.is_true(errors[1]:find("receiver_max_work_justification", 1, true) ~= nil)
  end,

  test_liveness_contract_rejects_under_budget_receiver_bound = function()
    local rows = copy_rows(core.restart_transition_table())
    local row = rows_by_state(rows)["merge-ready"]
    row.budget.minutes = 360
    local errors = core.liveness_contract_errors(rows)
    t.eq(#errors, 1)
    t.is_true(errors[1]:find("merge-ready", 1, true) ~= nil)
    t.is_true(errors[1]:find("budget.minutes", 1, true) ~= nil)
  end,

  test_liveness_contract_rejects_live_defer_without_resolver_or_existing_family = function()
    local rows = copy_rows(core.restart_transition_table())
    local row = rows_by_state(rows).reviewing
    row.liveness_contract.signal.family = "missing-family"
    row.liveness_contract.signal.resolver = "missing-resolver"
    row.liveness_contract.signal.max_age_minutes = nil
    local errors = core.liveness_contract_errors(rows)
    local joined = table.concat(errors, "\n")
    t.is_true(joined:find("missing-family", 1, true) ~= nil)
    t.is_true(joined:find("missing-resolver", 1, true) ~= nil)
    t.is_true(joined:find("max_age_minutes", 1, true) ~= nil)
    t.is_true(joined:find("resolver mismatch", 1, true) ~= nil)
  end,

  test_liveness_contract_rejects_live_defer_without_producer_binding = function()
    local rows = copy_rows(core.restart_transition_table())
    local row = rows_by_state(rows).reviewing
    row.liveness_contract.signal.producer = nil
    local errors = core.liveness_contract_errors(rows)
    t.eq(#errors, 1)
    t.is_true(errors[1]:find("reviewing", 1, true) ~= nil)
    t.is_true(errors[1]:find("producer binding", 1, true) ~= nil)
  end,

  test_liveness_contract_rejects_live_defer_family_resolver_producer_mismatch = function()
    local rows = copy_rows(core.restart_transition_table())
    local row = rows_by_state(rows).reviewing
    row.liveness_contract.signal.family = "converge-round"
    row.liveness_contract.signal.producer = "review-converge-round"
    local errors = core.liveness_contract_errors(rows)
    local joined = table.concat(errors, "\n")
    t.is_true(joined:find("producer binding family mismatch", 1, true) ~= nil)
    t.is_true(joined:find("producer binding resolver mismatch", 1, true) ~= nil)
  end,

  test_liveness_timeout_versions_preserve_lineage_and_attempts = function()
    local row = table_by_state()["impl-failed"]
    local base = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local state = {
      state = "impl-failed",
      version = base,
      marker_created_at = "2026-06-03T01:02:03Z",
    }
    local decision = core.liveness_timeout_decision(row, state, core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"))
    t.eq(decision.action, "redrive")
    t.eq(decision.attempt, 1)
    t.eq(core.version_timeout_round(decision.version, "impl-failed"), 1)
    t.eq(core.strip_transition_version_suffixes(decision.version), core.strip_transition_version_suffixes(base))
    local over = {
      state = "impl-failed",
      version = base .. "/timeout/impl-failed/3",
      marker_created_at = "2026-06-03T01:02:03Z",
    }
    local escalated = core.liveness_timeout_decision(row, over, core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"))
    t.eq(escalated.action, "escalate")
  end,

  test_replay_timeout_classification_counts_declines_as_stuck = function()
    local declined = {
      "skip-idempotent(retry-limit)",
      "skip-foreign(decomposed)",
      "skip-foreign(pr-link)",
      "skip-pending(no-attempt-marker)",
      "skip-pending(attempt-live)",
      "skip-stale(head-advanced)",
    }
    for _, outcome in ipairs(declined) do
      local previous = core.replay_from_table
      core.replay_from_table = function()
        if core._replay_skip_capture ~= nil then
          core._replay_skip_capture.outcome = outcome
          core._replay_skip_capture.reason = "declined"
        end
        return false
      end
      local ok, classified = pcall(function()
        return core.replay_from_table_classified("test", {}, { state = "ready" }, core.restart_transition_row("ready"), {})
      end)
      core.replay_from_table = previous
      if not ok then error(classified) end
      t.eq(classified.kind, "stuck")
      t.eq(classified.outcome, outcome)
    end
  end,

  test_live_thinking_converge_round_defers_timeout_count = function()
    local row = table_by_state().thinking
    local version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local source_ref = core.issue_source_ref("owner/repo", 42)
    local raised = capture_raises(function()
      local applied = core.maybe_timeout_redrive_from_table("liveness_scan", {
        repo = "owner/repo",
        number = 42,
        source_ref = source_ref,
      }, {
        state = "thinking",
        version = version,
        proposal_id = "github-devloop/issue/owner/repo/42",
        marker_created_at = "2026-06-03T00:00:00Z",
      }, row, {
        proposal_id = "github-devloop/issue/owner/repo/42",
        source_ref = source_ref,
        current = {
          comments = {
            {
              body = core.converge_round_marker("github-devloop/issue/owner/repo/42", version, core.source_ref_digest(source_ref), 1, "consensus:github-devloop/issue/owner/repo/42/loop/1", "Still converging", {
                { angle = "minimal", verdict = "continue", digest = "recent" },
              }),
              author_login = "fkst-test-bot",
              created_at = "2026-06-04T00:30:00Z",
            },
          },
        },
        now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"),
      })
      t.eq(applied, true)
    end)
    t.eq(#raised, 0)
  end,

  test_stale_thinking_converge_round_climbs_to_blocked_reconcile = function()
    local row = table_by_state().thinking
    local version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local source_ref = core.issue_source_ref("owner/repo", 42)
    local raised = capture_raises(function()
      local applied = core.maybe_timeout_redrive_from_table("liveness_scan", {
        repo = "owner/repo",
        number = 42,
        source_ref = source_ref,
      }, {
        state = "thinking",
        version = version .. "/timeout/thinking/3",
        proposal_id = "github-devloop/issue/owner/repo/42",
        marker_created_at = "2026-06-03T00:00:00Z",
      }, row, {
        proposal_id = "github-devloop/issue/owner/repo/42",
        source_ref = source_ref,
        current = {
          comments = {
            {
              body = core.converge_round_marker("github-devloop/issue/owner/repo/42", version, core.source_ref_digest(source_ref), 1, "consensus:github-devloop/issue/owner/repo/42/loop/1", "Stale convergence", {
                { angle = "minimal", verdict = "continue", digest = "stale" },
              }),
              author_login = "fkst-test-bot",
              created_at = "2026-06-03T00:00:00Z",
            },
          },
        },
        now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"),
      })
      t.eq(applied, true)
    end)
    t.eq(#raised, 1)
    t.eq(raised[1].queue, "devloop_timeout_reconcile")
    t.eq(raised[1].payload.state, "thinking")
  end,

  test_live_review_converge_round_defers_timeout_count = function()
    local row = table_by_state().reviewing
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local head_sha = "def456"
    local source_ref = core.pr_source_ref("owner/repo", 7)
    local review_proposal_id = core.pr_review_proposal_id("owner/repo", 7, version, head_sha)
    local raised = capture_raises(function()
      local applied = core.maybe_timeout_redrive_from_table("liveness_scan", {
        repo = "owner/repo",
        number = 42,
        source_ref = core.issue_source_ref("owner/repo", 42),
      }, {
        state = "reviewing",
        version = version,
        proposal_id = "github-devloop/issue/owner/repo/42",
        marker_created_at = "2026-06-03T00:00:00Z",
      }, row, {
        proposal_id = "github-devloop/issue/owner/repo/42",
        source_ref = source_ref,
        review_proposal_id = review_proposal_id,
        head_sha = head_sha,
        current = {
          comments = {},
        },
        current_pr = {
          comments = {
            {
              body = core.review_converge_round_marker(review_proposal_id, "github-devloop/issue/owner/repo/42", version, head_sha, core.source_ref_digest(source_ref), 1, "consensus:" .. review_proposal_id .. "/review/loop/1", "Still reviewing", {
                { angle = "minimal", verdict = "continue", digest = "recent" },
              }),
              author_login = "fkst-test-bot",
              created_at = "2026-06-04T00:30:00Z",
            },
          },
        },
        now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"),
      })
      t.eq(applied, true)
    end)
    t.eq(#raised, 0)
  end,

  test_stale_review_converge_round_climbs_to_blocked_reconcile = function()
    local row = table_by_state().reviewing
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local head_sha = "def456"
    local source_ref = core.pr_source_ref("owner/repo", 7)
    local review_proposal_id = core.pr_review_proposal_id("owner/repo", 7, version, head_sha)
    local raised = capture_raises(function()
      local applied = core.maybe_timeout_redrive_from_table("liveness_scan", {
        repo = "owner/repo",
        number = 42,
        source_ref = core.issue_source_ref("owner/repo", 42),
      }, {
        state = "reviewing",
        version = version .. "/timeout/reviewing/3",
        proposal_id = "github-devloop/issue/owner/repo/42",
        marker_created_at = "2026-06-03T00:00:00Z",
      }, row, {
        proposal_id = "github-devloop/issue/owner/repo/42",
        source_ref = source_ref,
        review_proposal_id = review_proposal_id,
        head_sha = head_sha,
        current = {
          comments = {},
        },
        current_pr = {
          comments = {
            {
              body = core.review_converge_round_marker(review_proposal_id, "github-devloop/issue/owner/repo/42", version, head_sha, core.source_ref_digest(source_ref), 1, "consensus:" .. review_proposal_id .. "/review/loop/1", "Stale review", {
                { angle = "minimal", verdict = "continue", digest = "stale" },
              }),
              author_login = "fkst-test-bot",
              created_at = "2026-06-03T00:00:00Z",
            },
          },
        },
        now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"),
      })
      t.eq(applied, true)
    end)
    t.eq(#raised, 1)
    t.eq(raised[1].queue, "devloop_timeout_reconcile")
    t.eq(raised[1].payload.state, "reviewing")
  end,

  test_merge_ready_within_ci_sla_waits_and_past_sla_climbs = function()
    local row = table_by_state()["merge-ready"]
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local fresh = capture_raises(function()
      local applied = core.maybe_timeout_redrive_from_table("liveness_scan", {
        repo = "owner/repo",
        number = 42,
        source_ref = core.issue_source_ref("owner/repo", 42),
      }, {
        state = "merge-ready",
        version = version,
        proposal_id = "github-devloop/issue/owner/repo/42",
        marker_created_at = "2026-06-03T20:00:00Z",
      }, row, {
        proposal_id = "github-devloop/issue/owner/repo/42",
        source_ref = core.pr_source_ref("owner/repo", 7),
        now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"),
      })
      t.eq(applied, false)
    end)
    t.eq(#fresh, 0)

    local stale = capture_raises(function()
      local applied = core.maybe_timeout_redrive_from_table("liveness_scan", {
        repo = "owner/repo",
        number = 42,
        source_ref = core.issue_source_ref("owner/repo", 42),
      }, {
        state = "merge-ready",
        version = version .. "/timeout/merge-ready/3",
        proposal_id = "github-devloop/issue/owner/repo/42",
        marker_created_at = "2026-06-03T00:00:00Z",
      }, row, {
        proposal_id = "github-devloop/issue/owner/repo/42",
        source_ref = core.pr_source_ref("owner/repo", 7),
        now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"),
      })
      t.eq(applied, true)
    end)
    t.eq(#stale, 1)
    t.eq(stale[1].queue, "devloop_timeout_reconcile")
    t.eq(stale[1].payload.state, "merge-ready")
  end,

  test_liveness_timeout_escalates_thinking_to_timeout_reconcile_event = function()
    local row = table_by_state().thinking
    local base = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local raised = {}
    local original_log_raise = core.log_raise
    core.log_raise = function(_, _, queue, payload)
      table.insert(raised, { queue = queue, payload = payload })
    end
    local ok, err = pcall(function()
      local applied = core.maybe_timeout_redrive_from_table("observe_issue", {
        repo = "owner/repo",
        number = 42,
        source_ref = core.issue_source_ref("owner/repo", 42),
      }, {
        state = "thinking",
        version = base .. "/timeout/thinking/3",
        proposal_id = "github-devloop/issue/owner/repo/42",
        marker_created_at = "2026-06-03T01:02:03Z",
      }, row, {
        proposal_id = "github-devloop/issue/owner/repo/42",
        now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"),
      })
      t.eq(applied, true)
    end)
    core.log_raise = original_log_raise
    if not ok then
      error(err)
    end
    t.eq(#raised, 1)
    t.eq(raised[1].queue, "devloop_timeout_reconcile")
    t.eq(raised[1].payload.schema, "github-devloop.timeout-reconcile.v1")
    t.eq(raised[1].payload.state, "thinking")
    t.eq(raised[1].payload.issue_version, base .. "/timeout/thinking/3")
    t.eq(raised[1].payload.round, 3)
    t.eq(raised[1].payload.dedup_key, "timeout-reconcile:" .. base .. "/timeout/thinking/3/timeout-reconcile/thinking/3")
  end,

  test_liveness_timeout_escalates_reviewing_to_timeout_reconcile_event = function()
    local row = table_by_state().reviewing
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local head_sha = "def456"
    local review_proposal_id = core.pr_review_proposal_id("owner/repo", 7, version, head_sha)
    local raised = {}
    local original_log_raise = core.log_raise
    core.log_raise = function(_, _, queue, payload)
      table.insert(raised, { queue = queue, payload = payload })
    end
    local ok, err = pcall(function()
      local applied = core.maybe_timeout_redrive_from_table("observe_pr", {
        repo = "owner/repo",
        number = 42,
        source_ref = core.issue_source_ref("owner/repo", 42),
      }, {
        state = "reviewing",
        version = version .. "/timeout/reviewing/3",
        proposal_id = "github-devloop/issue/owner/repo/42",
        marker_created_at = "2026-06-03T01:02:03Z",
      }, row, {
        proposal_id = "github-devloop/issue/owner/repo/42",
        source_ref = core.pr_source_ref("owner/repo", 7),
        review_proposal_id = review_proposal_id,
        head_sha = head_sha,
        now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"),
      })
      t.eq(applied, true)
    end)
    core.log_raise = original_log_raise
    if not ok then
      error(err)
    end
    t.eq(#raised, 1)
    t.eq(raised[1].queue, "devloop_timeout_reconcile")
    t.eq(raised[1].payload.schema, "github-devloop.timeout-reconcile.v1")
    t.eq(raised[1].payload.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(raised[1].payload.state, "reviewing")
    t.eq(raised[1].payload.issue_version, version .. "/timeout/reviewing/3")
    t.eq(raised[1].payload.round, 3)
  end,

  test_liveness_timeout_escalation_has_observable_event_for_every_non_terminal_row = function()
    local original_log_raise = core.log_raise
    local raised = {}
    core.log_raise = function(_, _, queue, payload)
      table.insert(raised, { queue = queue, payload = payload })
    end
    local ok, err = pcall(function()
      for _, row in ipairs(core.restart_transition_table()) do
        if row.terminal == false then
          local before = #raised
          local base = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
          local state = {
            state = row.from_state,
            version = base .. "/timeout/" .. row.from_state .. "/3",
            proposal_id = "github-devloop/issue/owner/repo/42",
            marker_created_at = "2026-06-03T01:02:03Z",
          }
          local applied = core.maybe_timeout_redrive_from_table("observe_issue", {
            repo = "owner/repo",
            number = 42,
            source_ref = core.issue_source_ref("owner/repo", 42),
          }, state, row, {
            proposal_id = state.proposal_id,
            source_ref = row.from_state == "reviewing" and core.pr_source_ref("owner/repo", 7) or core.issue_source_ref("owner/repo", 42),
            review_proposal_id = core.pr_review_proposal_id("owner/repo", 7, state.version, "def456"),
            head_sha = "def456",
            now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"),
          })
          t.eq(applied, true)
          t.eq(#raised, before + 1)
          if row.from_state == "blocked" then
            t.eq(raised[#raised].queue, "github-proxy.github_issue_comment_request")
            t.is_true(tostring(raised[#raised].payload.body or ""):find("fkst:github-devloop:decompose-exhausted:v1", 1, true) ~= nil)
          else
            t.eq(raised[#raised].queue, "devloop_timeout_reconcile")
            t.eq(raised[#raised].queue == row.driving_queue, false)
            t.eq(tostring(raised[#raised].payload.dedup_key or ""):find("/timeout/" .. row.from_state .. "/4", 1, true), nil)
          end
        end
      end
    end)
    core.log_raise = original_log_raise
    if not ok then
      error(err)
    end
  end,

  test_restart_table_matches_state_graph_and_stage_rank = function()
    local by_state = table_by_state()
    local expected = {
      thinking = true,
      ready = true,
      implementing = true,
      ["impl-failed"] = true,
      ["pr-open"] = true,
      reviewing = true,
      ["merge-ready"] = true,
      merging = true,
      fixing = true,
      ["review-meta"] = true,
      ["impl-failed"] = true,
      blocked = true,
      merged = true,
    }
    for state, next_states in pairs(core._state_graph) do
      if expected[state] then
        local row = by_state[state]
        t.is_true(row ~= nil)
        for _, next_state in ipairs(row.to_states) do
          t.is_true(has_value(next_states, next_state) or allowed_extra_transition(state, next_state))
        end
        t.is_true(core.stage_rank(state) > 0)
      end
    end
    for state in pairs(expected) do
      t.is_true(by_state[state] ~= nil)
    end
  end,

  test_restart_required_facts_declare_freshness_modes = function()
    for _, row in ipairs(core.restart_transition_table()) do
      if row.terminal == true then
        goto continue
      end
      local saw_marker = false
      for _, required in ipairs(row.required_facts) do
        t.is_true(type(required.family) == "string" and required.family ~= "")
        t.is_true(required.freshness == "marker-read" or required.freshness == "fetch-before-compare")
        if required.freshness == "marker-read" then
          saw_marker = true
        end
      end
      t.is_true(saw_marker)
      ::continue::
    end
  end,

  test_restart_payload_fields_are_covered_by_durable_fields = function()
    local errors = core.restart_field_coverage_errors()
    t.eq(#errors, 0)
  end,

  test_multi_effect_rows_declare_and_call_completeness_derivation = function()
    local by_state = table_by_state()
    t.eq(by_state.ready.effects.intent_count, 3)
    t.eq(by_state.ready.effects.kinds[1], "result-marker")
    t.eq(by_state.ready.effects.kinds[2], "ready-label")
    t.eq(by_state.ready.effects.kinds[3], "devloop_ready")
    t.eq(by_state.ready.effects.completeness_derivation, "result_effects_complete")
    t.eq(by_state.blocked.effects.intent_count, 2)
    t.eq(by_state.blocked.effects.completeness_derivation, "decompose_children_complete")
    t.eq(#core.restart_effect_contract_errors(), 0)
  end,

  test_pr_side_rows_declare_pr_state_label_projection = function()
    local by_state = table_by_state()
    for _, state in ipairs({ "pr-open", "reviewing", "merge-ready", "merging", "fixing" }) do
      local row = by_state[state]
      t.is_true(row ~= nil)
      t.is_true(has_value(row.effects.kinds, "pr-state-label"), state .. " missing pr-state-label effect")
      t.is_true(
        tostring(row.effects.completeness or ""):find("PR-local state label projection", 1, true) ~= nil,
        state .. " missing PR-local label completeness text"
      )
    end
  end,

  test_multi_effect_contract_rejects_marker_only_rows = function()
    local rows = copy_rows(core.restart_transition_table())
    local ready = rows_by_state(rows).ready
    ready.effects.completeness_derivation = nil
    local errors = core.restart_effect_contract_errors(rows)
    t.eq(#errors, 1)
    t.is_true(errors[1]:find("ready", 1, true) ~= nil)
    t.is_true(errors[1]:find("completeness derivation", 1, true) ~= nil)
  end,

  test_restart_field_coverage_catches_374_shape_missing_gate_baseline = function()
    local rows = copy_rows(core.restart_transition_table())
    rows_by_state(rows).fixing.payload_fields.gate_baseline_sha = nil
    local errors = core.restart_field_coverage_errors(rows)
    t.eq(#errors, 1)
    t.is_true(errors[1]:find("fixing.gate_baseline_sha", 1, true) ~= nil)
    t.is_true(errors[1]:find("missing required replay payload field", 1, true) ~= nil)
  end,

  test_declared_marker_fields_exist_in_marker_builders = function()
    local parsed = parse_marker_builders(marker_builder_paths())
    for family, attrs in pairs(core.restart_durable_marker_fields()) do
      t.is_true(parsed[family] ~= nil, "missing marker family " .. tostring(family))
      for attr in pairs(attrs) do
        t.is_true(parsed[family][attr] == true, "missing marker attr " .. tostring(family) .. "." .. tostring(attr))
      end
    end
  end,

  test_source_ref_derivations_are_declared = function()
    local derivations = core.restart_source_ref_derivations()
    t.eq(derivations.issue, true)
    t.eq(derivations.pr, true)
    t.eq(derivations.entity, true)
  end,

  test_replay_payload_fields_resolve_from_declared_table_map = function()
    local state = {
      state = "fixing",
      version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-12T00-00-00Z",
    }
    local fields = core.resolve_replay_payload_fields(table_by_state().fixing, state, {
      issue = {
        repo = "owner/repo",
        source_ref = core.issue_source_ref("owner/repo", 42),
      },
      proposal_id = "github-devloop/issue/owner/repo/42",
      link = {
        pr_number = 7,
      },
      feedback = {
        review_proposal_id = "github-devloop/pr-review/owner/repo/7/v/def456",
        review_dedup_key = "consensus:github-devloop/pr-review/owner/repo/7/v/def456/review",
        reviewed_head_sha = "def456",
        gate_baseline_sha = "abc123",
      },
    })
    t.eq(fields.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(fields.pr_number, 7)
    t.eq(fields.version, state.version)
    t.eq(fields.review_proposal_id, "github-devloop/pr-review/owner/repo/7/v/def456")
    t.eq(fields.review_dedup_key, "consensus:github-devloop/pr-review/owner/repo/7/v/def456/review")
    t.eq(fields.reviewed_head_sha, "def456")
    t.eq(fields.gate_baseline_sha, "abc123")
    t.eq(fields.source_ref.ref, "owner/repo#pr/7")
  end,

  test_replayer_gathers_fetch_before_compare_pr_facts_from_table = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-12T00-00-00Z"
    local issue = {
      repo = "owner/repo",
      number = 42,
      source_ref = core.issue_source_ref("owner/repo", 42),
    }
    local state = {
      state = "pr-open",
      version = version,
    }
    local issue_comments = {
      { body = core.state_marker(proposal_id, "pr-open", version), author_login = "fkst-test-bot" },
      { body = core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", version, "dev"), author_login = "fkst-test-bot" },
    }
    t.mock_command(core.gh_pr_view_observe_cmd("owner/repo", 7), {
      stdout = '{"headRefName":"devloop-owner-repo-42-01HY","headRefOid":"def456","baseRefName":"dev","state":"OPEN","updatedAt":"2026-06-03T02:03:04Z","comments":[]}\n',
      stderr = "",
      exit_code = 0,
    })
    local gathered = core.gather_replay_required_facts(table_by_state()["pr-open"], issue, state, {
      proposal_id = proposal_id,
      current = { comments = issue_comments },
      snapshot = {
        comments = issue_comments,
        prs = {
          {
            number = 7,
            current = {
              head_sha = "stale",
              head_ref_name = "stale",
              base_ref_name = "dev",
              state = "OPEN",
              comments = {},
            },
          },
        },
      },
    })
    t.eq(gathered.snapshot.prs[1].current.head_sha, "def456")
    t.eq(#t.command_calls(), 1)
  end,

  test_replayer_fetch_before_compare_ignores_caller_fresh_flag = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-12T00-00-00Z"
    local issue = {
      repo = "owner/repo",
      number = 42,
      source_ref = core.issue_source_ref("owner/repo", 42),
    }
    local state = {
      state = "pr-open",
      version = version,
    }
    local issue_comments = {
      { body = core.state_marker(proposal_id, "pr-open", version), author_login = "fkst-test-bot" },
      { body = core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", version, "dev"), author_login = "fkst-test-bot" },
    }
    t.mock_command(core.gh_pr_view_observe_cmd("owner/repo", 7), {
      stdout = '{"headRefName":"devloop-owner-repo-42-01HY","headRefOid":"def456","baseRefName":"dev","state":"OPEN","updatedAt":"2026-06-03T02:03:04Z","comments":[]}\n',
      stderr = "",
      exit_code = 0,
    })
    local gathered = core.gather_replay_required_facts(table_by_state()["pr-open"], issue, state, {
      proposal_id = proposal_id,
      current = { comments = issue_comments },
      snapshot = {
        fresh = true,
        fetch_before_compare = {
          ["pr-head"] = true,
        },
        comments = issue_comments,
        prs = {
          {
            number = 7,
            current = {
              head_sha = "stale",
              head_ref_name = "stale",
              base_ref_name = "dev",
              state = "OPEN",
              comments = {},
            },
          },
        },
      },
    })
    t.eq(gathered.snapshot.prs[1].current.head_sha, "def456")
    t.eq(#t.command_calls(), 1)
  end,

  test_observe_issue_replay_is_table_driven = function()
    local text = file.read("packages/github-devloop/departments/observe_issue/main.lua")
    t.is_true(text:find("core.replay_from_table", 1, true) ~= nil)
    t.eq(text:find("build_replayed_fixing_payload", 1, true), nil)
    t.eq(text:find("build_devloop_review_meta_payload", 1, true), nil)
    t.eq(text:find("build_decompose_replay_payload", 1, true), nil)
    t.eq(text:find("build_devloop_reviewing_payload", 1, true), nil)
  end,

  test_observe_pr_replay_is_table_driven = function()
    local text = file.read("packages/github-devloop/departments/observe_pr/main.lua")
    t.is_true(text:find("core.replay_from_table", 1, true) ~= nil)
    t.eq(text:find("build_replayed_fixing_payload", 1, true), nil)
    t.eq(text:find("build_decompose_replay_payload", 1, true), nil)
    t.eq(text:find("build_devloop_merge_ready_payload", 1, true), nil)
    t.eq(text:find("review_carry_over_marker", 1, true), nil)
  end,
}
