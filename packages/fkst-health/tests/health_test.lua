-- Unit tests for the pure evidence core: one test per decision rule, the precedence
-- cases where signals disagree, the partial-evidence rule that keeps `unknown` from
-- being reachable by a single failed probe, and a totality sweep proving the core
-- returns a valid v1 verdict for every input shape without ever erroring.
local health = require("health")
local t = fkst.test

local function unreadable(why)
  return { readable = false, why = why }
end

local function evidence_value(verdict, key)
  for _, entry in ipairs(verdict.evidence) do
    if entry.key == key then
      return entry.value
    end
  end
  return nil
end

-- Every probe readable and quiet: the baseline a test mutates one signal away from.
local function quiet()
  return {
    deliveries = { readable = true, completed_delta = 0, in_flight = 0, retrying = 0, depth = 0, dead_letters = 0, dead_letter_delta = 0 },
    codex = { readable = true, runs_started = 0, runs_finished = 0, running = 0 },
    repository = { readable = true, commits = 0, new_branches = 0, new_pull_requests = 0 },
    work_items = { readable = true, open = 2, closed_delta = 0, items = {} },
    faults = { readable = true, recurring = 0, framework_erroring = false },
    window = { consecutive_no_progress = 0 },
  }
end

return {
  -- ---- rule 1: no open work items -> idle ----------------------------------
  test_rule_one_no_open_work_items_is_idle = function()
    local observations = quiet()
    observations.work_items.open = 0
    t.eq(health.decide(observations).status, "idle")
  end,

  -- `idle` outranks everything, including a fault: a session with nothing left to do
  -- is about to be reaped and must not be reported as broken on the way out.
  test_rule_one_outranks_a_recurring_fault = function()
    local observations = quiet()
    observations.work_items.open = 0
    observations.faults = { readable = true, recurring = 9, framework_erroring = false }
    t.eq(health.decide(observations).status, "idle")
  end,

  -- ---- rule 2: framework erroring or DLQ growing -> failing ------------------
  test_rule_two_framework_erroring_is_failing = function()
    local observations = quiet()
    observations.faults = { readable = true, recurring = 1, framework_erroring = true, top_fault = "redb lock contention" }
    local verdict = health.decide(observations)
    t.eq(verdict.status, "failing")
    t.is_true(verdict.headline:find("redb lock contention", 1, true) ~= nil, verdict.headline)
  end,

  test_rule_two_growing_dead_letter_queue_is_failing = function()
    local observations = quiet()
    observations.deliveries.dead_letter_delta = 4
    t.eq(health.decide(observations).status, "failing")
  end,

  -- A non-growing dead-letter backlog is history, not a live failure.
  test_static_dead_letter_backlog_is_not_failing = function()
    local observations = quiet()
    observations.deliveries.dead_letters = 7
    observations.deliveries.dead_letter_delta = 0
    t.eq(health.decide(observations).status, "stalled")
  end,

  -- ---- rule 3: any progress signal moved -> working -------------------------
  test_rule_three_completed_deliveries_are_working = function()
    local observations = quiet()
    observations.deliveries.completed_delta = 3
    t.eq(health.decide(observations).status, "working")
  end,

  test_rule_three_a_new_commit_alone_is_working = function()
    local observations = quiet()
    observations.repository.commits = 1
    local verdict = health.decide(observations)
    t.eq(verdict.status, "working")
    t.is_true(verdict.progressed)
  end,

  test_rule_three_a_finished_codex_run_is_working = function()
    local observations = quiet()
    observations.codex.runs_finished = 1
    t.eq(health.decide(observations).status, "working")
  end,

  test_rule_three_a_closed_work_item_is_working = function()
    local observations = quiet()
    observations.work_items.closed_delta = 1
    t.eq(health.decide(observations).status, "working")
  end,

  -- ---- rule 4: output but a recurring fault -> blocked ----------------------
  test_rule_four_output_with_a_recurring_fault_is_blocked = function()
    local observations = quiet()
    observations.codex.runs_started = 2
    observations.faults = { readable = true, recurring = 5, framework_erroring = false, top_fault = "cargo build failed" }
    local verdict = health.decide(observations)
    t.eq(verdict.status, "blocked")
    t.is_true(verdict.headline:find("cargo build failed", 1, true) ~= nil, verdict.headline)
  end,

  -- Below the recurrence threshold a repeat is ordinary retry behaviour, not a wedge.
  test_rule_four_needs_the_fault_to_clear_the_recurrence_threshold = function()
    local observations = quiet()
    observations.codex.runs_started = 2
    observations.faults = { readable = true, recurring = health.fault_recurrence_threshold - 1, framework_erroring = false }
    t.eq(health.decide(observations).status, "stalled")
  end,

  -- A recurring fault with NO output at all is a stall, not a block: nothing is
  -- being produced for the fault to be obstructing.
  test_rule_four_needs_output_to_be_present = function()
    local observations = quiet()
    observations.faults = { readable = true, recurring = 9, framework_erroring = false }
    t.eq(health.decide(observations).status, "stalled")
  end,

  -- ---- rule 5: no progress, no new output -> stalled ------------------------
  test_rule_five_a_quiet_window_is_stalled = function()
    local verdict = health.decide(quiet())
    t.eq(verdict.status, "stalled")
    t.is_true(verdict.headline:find("No progress", 1, true) ~= nil, verdict.headline)
  end,

  -- ---- rule 6: nothing readable -> unknown ---------------------------------
  test_rule_six_a_total_probe_blackout_is_unknown = function()
    local verdict = health.decide({
      deliveries = unreadable("observe refused"),
      codex = unreadable("codex_runs unavailable"),
      repository = unreadable("git log failed"),
      work_items = unreadable("issue search failed"),
      faults = unreadable("observe refused"),
      window = { consecutive_no_progress = 1 },
    })
    t.eq(verdict.status, "unknown")
    t.eq(verdict.confidence, "low")
    t.eq(evidence_value(verdict, "deliveries_readable"), "false")
    t.eq(evidence_value(verdict, "deliveries_unreadable_why"), "observe refused")
  end,

  -- The failure mode this guards: a package that cries `unknown` whenever one probe
  -- hiccups. One unreadable probe plus one positive signal is `working`.
  test_partial_evidence_with_one_positive_signal_is_working_not_unknown = function()
    local verdict = health.decide({
      deliveries = unreadable("observe refused"),
      codex = unreadable("codex_runs unavailable"),
      repository = { readable = true, commits = 2, new_branches = 0, new_pull_requests = 0 },
      work_items = unreadable("issue search failed"),
      faults = unreadable("observe refused"),
    })
    t.eq(verdict.status, "working")
    t.eq(verdict.confidence, "medium")
  end,

  -- A single readable progress-bearing signal is enough to justify `stalled`; the
  -- verdict is not downgraded to `unknown` just because the other probes failed.
  test_one_readable_quiet_progress_signal_is_stalled_not_unknown = function()
    local verdict = health.decide({
      deliveries = unreadable("observe refused"),
      repository = { readable = true, commits = 0, new_branches = 0, new_pull_requests = 0 },
    })
    t.eq(verdict.status, "stalled")
  end,

  -- The one corner where a signal WAS read and the verdict is still `unknown`:
  -- `faults` cannot speak to forward movement, so a readable-and-quiet log alone is
  -- no basis for claiming a stall. Calling this `stalled` would raise a false alarm
  -- on every session whose progress probes happened to fail, so it stays `unknown`.
  test_only_the_fault_signal_readable_is_unknown_not_stalled = function()
    local verdict = health.decide({
      deliveries = unreadable("observe refused"),
      codex = unreadable("codex_runs unavailable"),
      repository = unreadable("git log failed"),
      work_items = unreadable("issue search failed"),
      faults = { readable = true, recurring = 0, framework_erroring = false },
    })
    t.eq(verdict.status, "unknown")
  end,

  -- ---- precedence where signals conflict -----------------------------------
  -- Progress AND a recurring fault is `working`: rule 3 precedes rule 4, so `blocked`
  -- is reserved for output that is NOT landing.
  test_precedence_progress_beats_a_recurring_fault = function()
    local observations = quiet()
    observations.repository.commits = 1
    observations.codex.runs_started = 3
    observations.faults = { readable = true, recurring = 12, framework_erroring = false }
    t.eq(health.decide(observations).status, "working")
  end,

  -- A framework error beats visible progress: partial output from an erroring engine
  -- is not a healthy session.
  test_precedence_failing_beats_progress = function()
    local observations = quiet()
    observations.repository.commits = 5
    observations.faults = { readable = true, recurring = 0, framework_erroring = true }
    t.eq(health.decide(observations).status, "failing")
  end,

  -- ---- stall confidence ----------------------------------------------------
  test_first_quiet_window_is_low_confidence = function()
    local observations = quiet()
    observations.window.consecutive_no_progress = 0
    local verdict = health.decide(observations)
    t.eq(verdict.status, "stalled")
    t.eq(verdict.confidence, "low")
  end,

  test_consecutive_quiet_windows_raise_confidence_to_high = function()
    local observations = quiet()
    observations.window.consecutive_no_progress = health.stall_confidence_windows
    local verdict = health.decide(observations)
    t.eq(verdict.status, "stalled")
    t.eq(verdict.confidence, "high")
    t.eq(evidence_value(verdict, "consecutive_no_progress_windows"), tostring(health.stall_confidence_windows))
  end,

  test_confidence_is_high_only_when_every_signal_was_readable = function()
    local observations = quiet()
    observations.repository.commits = 1
    t.eq(health.decide(observations).confidence, "high")
    observations.faults = unreadable("observe refused")
    t.eq(health.decide(observations).confidence, "medium")
  end,

  -- ---- bounds --------------------------------------------------------------
  test_work_items_are_bounded_and_malformed_rows_are_dropped = function()
    local observations = quiet()
    observations.work_items.items = { { number = "not-a-number", state = "open" }, 7, { state = "open" } }
    for index = 1, health.work_item_ceiling + 20 do
      table.insert(observations.work_items.items, { number = index, state = "open", progress = "none" })
    end
    local verdict = health.decide(observations)
    t.eq(#verdict.work_items, health.work_item_ceiling)
    t.eq(verdict.work_items[1].number, 1)
    t.eq(verdict.work_items[1].state, "open")
  end,

  test_headline_and_evidence_stay_within_the_v1_bounds = function()
    local observations = quiet()
    observations.faults = {
      readable = true,
      framework_erroring = true,
      top_fault = string.rep("very long fault text ", 200),
    }
    local verdict = health.decide(observations)
    t.is_true(#verdict.headline <= health.headline_character_ceiling, tostring(#verdict.headline))
    t.is_true(#verdict.evidence <= health.evidence_entry_ceiling, tostring(#verdict.evidence))
    t.is_true(verdict.headline:find("\n") == nil, verdict.headline)
  end,

  -- ---- totality ------------------------------------------------------------
  -- The core must return a valid v1 verdict for EVERY input, including shapes no
  -- probe would ever produce. It rides every session, so an error here is a
  -- fleet-wide missing heartbeat.
  test_core_is_total_over_empty_malformed_and_partial_evidence = function()
    local cases = {
      nil,
      42,
      "not a table",
      true,
      {},
      { deliveries = 5 },
      { deliveries = { readable = "yes" } },
      { deliveries = { readable = true, completed_delta = "many" } },
      { deliveries = { readable = true, completed_delta = -1 } },
      { deliveries = { readable = true, completed_delta = 0 / 0 } },
      { deliveries = { readable = true, completed_delta = math.huge } },
      { deliveries = { readable = true, completed_delta = 1.5 } },
      { work_items = { readable = true, open = 0 } },
      { work_items = { readable = true, items = "nope" } },
      { work_items = { readable = true, open = 1, items = { true, false, {} } } },
      { faults = { readable = true, framework_erroring = "true" } },
      { faults = { readable = true, recurring = 99, top_fault = { nested = true } } },
      { window = "broken" },
      { window = { consecutive_no_progress = -4 } },
      { codex = { readable = true, running = 2 }, faults = { readable = true, recurring = 99 } },
    }
    for index = 1, 20 do
      local case = cases[index]
      local ok, verdict = pcall(health.decide, case)
      t.is_true(ok, "case " .. tostring(index) .. " errored: " .. tostring(verdict))
      t.is_true(health.is_status(verdict.status), "case " .. tostring(index) .. " status=" .. tostring(verdict.status))
      t.is_true(type(verdict.headline) == "string" and #verdict.headline > 0, "case " .. tostring(index))
      t.is_true(#verdict.headline <= health.headline_character_ceiling, "case " .. tostring(index))
      t.is_true(#verdict.evidence <= health.evidence_entry_ceiling, "case " .. tostring(index))
      t.is_true(#verdict.work_items <= health.work_item_ceiling, "case " .. tostring(index))
    end
  end,

  -- A deterministic pseudo-random sweep over generated signal tables: same seed, same
  -- inputs, every run. Property under test: the verdict is always a valid v1 status
  -- and the core never errors.
  test_core_is_total_over_a_deterministic_fuzz_sweep = function()
    local seed = 20260730
    local function nextval(bound)
      seed = (seed * 1103515245 + 12345) % 2147483648
      return seed % bound
    end
    -- index 8 is deliberately past the end, so `nil` is one of the generated shapes.
    local shapes = { true, false, 0, 3, -2, "x", 0 / 0 }
    for _ = 1, 400 do
      local observations = {}
      for _, name in ipairs({ "deliveries", "codex", "repository", "work_items", "faults" }) do
        local pick = nextval(4)
        if pick == 0 then
          observations[name] = shapes[nextval(8) + 1]
        elseif pick == 1 then
          observations[name] = { readable = false, why = "probe " .. tostring(nextval(100)) }
        else
          observations[name] = {
            readable = true,
            completed_delta = shapes[nextval(8) + 1],
            runs_started = shapes[nextval(8) + 1],
            runs_finished = shapes[nextval(8) + 1],
            running = shapes[nextval(8) + 1],
            commits = shapes[nextval(8) + 1],
            new_branches = shapes[nextval(8) + 1],
            new_pull_requests = shapes[nextval(8) + 1],
            open = shapes[nextval(8) + 1],
            closed_delta = shapes[nextval(8) + 1],
            in_flight = shapes[nextval(8) + 1],
            retrying = shapes[nextval(8) + 1],
            dead_letter_delta = shapes[nextval(8) + 1],
            recurring = shapes[nextval(8) + 1],
            framework_erroring = shapes[nextval(8) + 1],
            top_fault = shapes[nextval(8) + 1],
            items = shapes[nextval(8) + 1],
          }
        end
      end
      observations.window = { consecutive_no_progress = shapes[nextval(8) + 1] }
      local ok, verdict = pcall(health.decide, observations)
      t.is_true(ok, "fuzz case errored: " .. tostring(verdict))
      t.is_true(health.is_status(verdict.status), "fuzz status=" .. tostring(verdict.status))
      t.is_true(#verdict.headline <= health.headline_character_ceiling, "fuzz headline too long")
    end
  end,

  test_is_status_accepts_only_the_v1_taxonomy = function()
    for _, status in ipairs(health.statuses) do
      t.is_true(health.is_status(status), status)
    end
    for _, bad in ipairs({ "Working", "healthy", "degraded", "", "ok" }) do
      t.eq(health.is_status(bad), false)
    end
    t.eq(health.is_status(nil), false)
    t.eq(health.is_status(7), false)
  end,

  -- ---- purity --------------------------------------------------------------
  -- Proves the core reaches no port and no ambient capability at call time: every
  -- primitive it could reach for is removed, and it still returns a valid verdict.
  test_core_runs_with_no_ports_and_no_process_or_file_capability = function()
    local saved = {
      os_execute = os.execute,
      io_open = io.open,
      exec_sync = exec_sync,
      exec_argv = exec_argv,
      spawn_codex_sync = spawn_codex_sync,
      file = file,
      raise = raise,
      truncate_utf8 = truncate_utf8,
    }
    os.execute = nil
    io.open = nil
    exec_sync = nil
    exec_argv = nil
    spawn_codex_sync = nil
    file = nil
    raise = nil
    truncate_utf8 = nil

    local observations = quiet()
    observations.faults = { readable = true, framework_erroring = true, top_fault = string.rep("x", 900) }
    local ok, verdict = pcall(health.decide, observations)

    os.execute = saved.os_execute
    io.open = saved.io_open
    exec_sync = saved.exec_sync
    exec_argv = saved.exec_argv
    spawn_codex_sync = saved.spawn_codex_sync
    file = saved.file
    raise = saved.raise
    truncate_utf8 = saved.truncate_utf8

    t.is_true(ok, tostring(verdict))
    t.eq(verdict.status, "failing")
    t.is_true(#verdict.headline <= health.headline_character_ceiling, tostring(#verdict.headline))
  end,
}
