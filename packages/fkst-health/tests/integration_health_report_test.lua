-- End-to-end tests for the health_report department against fake ports.
--
-- The property under test throughout is the failure posture: this department ships in
-- the default manifest, so it rides every session on the platform. A refused probe, a
-- codex that never answers, a filesystem that will not take the write -- none of them
-- may fail the tick, and none of them may cost the report its rules-derived verdict.
-- testkit's run_fake errors loudly if the pipeline throws, so every test below is
-- also an assertion that the tick survived.
local env_port = require("departments.health_report.env_port")
local helper = require("tests.health_report_helpers")
local testing = require("testkit.testing")
local writer = require("departments.health_report.writer")
local t = fkst.test

local function calls_of(department, op)
  local out = {}
  for _, call in ipairs(department.fs.calls) do
    if call.op == op then
      table.insert(out, call)
    end
  end
  return out
end

local function scalar(matter, key)
  return matter:match("\n?" .. key .. ' = "?([^"\n]*)"?')
end

return {
  -- ---- the happy path -------------------------------------------------------
  test_a_tick_writes_exactly_one_report_with_the_contract_filename = function()
    local department = helper.department()
    local result = testing.run_fake(department, helper.tick())

    local report = helper.only_report(department)
    t.eq(report.path, helper.health_directory .. "/" .. report.name)
    t.eq(
      report.name,
      helper.namespace .. "-" .. helper.session .. "-health-agent-status-report-20260802-060000.md"
    )
    t.is_true(report.name:find(":") == nil, report.name)

    local matter = helper.front_matter(report.text)
    t.is_true(matter:find("fkst_health_report = 1", 1, true) ~= nil, matter)
    t.eq(scalar(matter, "session_id"), helper.session)
    t.eq(scalar(matter, "namespace"), helper.namespace)
    t.eq(scalar(matter, "producer"), "fkst-health@0.1.0")
    t.eq(scalar(matter, "generated_at"), "2026-08-02T06:00:00Z")
    t.eq(scalar(matter, "expected_interval_secs"), "600")
    t.is_true(helper.body(report.text):find("The session is quiet.", 1, true) ~= nil, report.text)

    -- The broadcast a sibling package could subscribe to.
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "health_report_written")
    t.eq(result.raises[1].payload.schema, "fkst-health.report-written.v1")
    t.eq(result.raises[1].payload.session_id, helper.session)
    t.eq(result.raises[1].payload.source_ref.kind, "cron")
  end,

  test_with_the_namespace_unset_the_prefix_and_its_hyphen_are_omitted = function()
    local department = helper.department({
      env = {
        FKST_RUNTIME_ROOT = helper.runtime_root,
        FKST_SESSION_ID = helper.session,
      },
    })
    testing.run_fake(department, helper.tick())

    local report = helper.only_report(department)
    t.eq(report.name, helper.session .. "-health-agent-status-report-20260802-060000.md")
    t.is_true(report.name:sub(1, 1) ~= "-", report.name)
    t.eq(helper.front_matter(report.text):find("namespace", 1, true), nil)
  end,

  -- ---- the codex narrates; the rules decide ---------------------------------
  -- The single most important behavioural guarantee: a judge that returns a competing
  -- verdict cannot change what a user is shown.
  test_a_codex_returning_a_contradictory_status_does_not_change_the_emitted_status = function()
    local department = helper.department({
      run_codex = function()
        return {
          stdout = "status = \"working\"\nEverything is fine and three PRs merged.",
          stderr = "",
          exit_code = 0,
        }
      end,
    })
    testing.run_fake(department, helper.tick())

    local report = helper.only_report(department)
    local matter = helper.front_matter(report.text)
    -- Quiet observe, no commits, no work-item query: the rules say `stalled`.
    t.eq(scalar(matter, "status"), "stalled")
    -- The reply survives verbatim in the BODY, where it is opaque prose.
    t.is_true(helper.body(report.text):find("Everything is fine", 1, true) ~= nil, report.text)
  end,

  test_the_judgment_codex_is_spawned_read_only_from_the_project_checkout = function()
    local department = helper.department()
    testing.run_fake(department, helper.tick())

    local spawns = calls_of(department, "codex")
    t.eq(#spawns, 1)
    t.eq(spawns[1].opts.sandbox, "read-only")
    -- "." and not a scratch dir: a read-only-sandbox codex refuses to start outside a
    -- git repository, so it reads its evidence from an absolute context path instead.
    t.eq(spawns[1].opts.worktree, ".")
    t.is_true(spawns[1].opts.prompt:find(helper.health_directory, 1, true) ~= nil, spawns[1].opts.prompt)
    -- No identity fields, which is what keeps a raw spawn compliant with the
    -- live-run-dispatch ratchet.
    t.eq(spawns[1].opts.role, nil)
    t.eq(spawns[1].opts.proposal_id, nil)
    t.eq(spawns[1].opts.dedup_key, nil)
  end,

  -- ---- codex failure modes: a report every time -----------------------------
  test_codex_failure_timeout_and_empty_output_each_still_produce_a_valid_report = function()
    local cases = {
      { name = "spawn error", codex = function() error("boom") end, why = "codex spawn failed" },
      { name = "timeout", codex = function() return { stdout = "", stderr = "", exit_code = 124 } end, why = "codex timed out" },
      { name = "nonzero", codex = function() return { stdout = "", stderr = "", exit_code = 3 } end, why = "codex exited non-zero" },
      { name = "empty", codex = function() return { stdout = "   \n", stderr = "", exit_code = 0 } end, why = "codex produced no narrative" },
      { name = "no result", codex = function() return nil end, why = "codex returned no result" },
    }
    for _, case in ipairs(cases) do
      local department = helper.department({ run_codex = case.codex })
      testing.run_fake(department, helper.tick())

      local report = helper.only_report(department)
      local matter = helper.front_matter(report.text)
      t.is_true(matter:find("fkst_health_report = 1", 1, true) ~= nil, case.name)
      t.eq(scalar(matter, "status"), "stalled")
      t.eq(scalar(matter, "session_id"), helper.session)
      local body = helper.body(report.text)
      t.is_true(body:find("narrative summary is unavailable", 1, true) ~= nil, case.name .. ": " .. body)
      t.is_true(body:find(case.why, 1, true) ~= nil, case.name .. ": " .. body)
    end
  end,

  -- ---- atomicity ------------------------------------------------------------
  test_the_report_is_published_by_rename_and_leaves_no_partial_behind = function()
    local department = helper.department()
    testing.run_fake(department, helper.tick())

    local report = helper.only_report(department)
    -- The final name is NEVER written directly; it only ever appears via the rename.
    for _, call in ipairs(calls_of(department, "write")) do
      t.is_true(call.path ~= report.path, "final name was written in place: " .. call.path)
    end

    local moves = calls_of(department, "mv")
    t.eq(#moves, 1)
    t.eq(moves[1].argv[4], report.path)
    local partial = moves[1].argv[3]
    t.is_true(partial:sub(-#writer.partial_suffix) == writer.partial_suffix, partial)
    -- The temporary name is a dotfile ending in .partial: the control plane's parser
    -- rejects both, so an interrupted tick cannot leave a collectable artefact.
    t.is_true((partial:match("([^/]+)$")):sub(1, 1) == ".", partial)
    t.eq(department.fs.entries[partial], nil)
    for path in pairs(department.fs.entries) do
      t.is_true(path:sub(-#writer.partial_suffix) ~= writer.partial_suffix, "partial left behind: " .. path)
    end
  end,

  test_a_failed_rename_publishes_nothing_and_still_does_not_fail_the_tick = function()
    local fs = helper.fake_fs()
    local underlying = fs.exec_argv
    fs.exec_argv = function(request)
      local argv = type(request) == "table" and request.argv or {}
      if tostring(argv[1]) == "mv" then
        table.insert(fs.calls, { op = "mv", argv = argv })
        return { stdout = "", stderr = "read-only filesystem", exit_code = 1 }
      end
      return underlying(request)
    end
    local department = helper.department({ fs = fs })
    local result = testing.run_fake(department, helper.tick())

    t.eq(#helper.reports(department), 0)
    t.eq(#result.raises, 0)
    for path in pairs(department.fs.entries) do
      t.is_true(path:sub(-#writer.partial_suffix) ~= writer.partial_suffix, "partial left behind: " .. path)
    end
  end,

  -- ---- retention ------------------------------------------------------------
  test_reports_beyond_the_newest_two_hundred_are_pruned = function()
    local fs = helper.fake_fs()
    for index = 1, 250 do
      local stamp = string.format("202601%02d-%06d", (index % 28) + 1, index)
      fs.entries[helper.health_directory
        .. "/"
        .. helper.namespace
        .. "-"
        .. helper.session
        .. "-health-agent-status-report-"
        .. stamp
        .. ".md"] = "+++\n+++\nold\n"
    end
    local department = helper.department({ fs = fs })
    testing.run_fake(department, helper.tick())

    local reports = helper.reports(department)
    t.eq(#reports, writer.retention)
    -- The tick's own report is the newest and must have survived the sweep.
    local kept = false
    for _, report in ipairs(reports) do
      if report.name:find("20260802-060000", 1, true) ~= nil then
        kept = true
      end
    end
    t.is_true(kept, "the report just written was pruned")
  end,

  test_prune_ignores_files_that_are_not_reports = function()
    local fs = helper.fake_fs()
    fs.entries[helper.health_directory .. "/notes.txt"] = "keep me"
    fs.entries[helper.health_directory .. "/.hidden.md"] = "keep me too"
    for index = 1, 250 do
      fs.entries[helper.health_directory
        .. "/"
        .. helper.session
        .. "-health-agent-status-report-2026010"
        .. tostring(index % 9 + 1)
        .. "-"
        .. string.format("%06d", index)
        .. ".md"] = "+++\n+++\n"
    end
    local department = helper.department({ fs = fs })
    testing.run_fake(department, helper.tick())

    t.eq(fs.entries[helper.health_directory .. "/notes.txt"], "keep me")
    t.eq(fs.entries[helper.health_directory .. "/.hidden.md"], "keep me too")
  end,

  -- ---- degraded probes ------------------------------------------------------
  test_a_single_failed_probe_degrades_one_signal_and_still_produces_a_report = function()
    local department = helper.department({
      observe = function()
        error("fkst-health: observe-unavailable: snapshot refused")
      end,
      commit_count = function()
        return 4
      end,
    })
    testing.run_fake(department, helper.tick())

    local matter = helper.front_matter(helper.only_report(department).text)
    -- The commit signal survived, so the verdict is `working` -- not `unknown`.
    t.eq(scalar(matter, "status"), "working")
    t.is_true(matter:find('key = "deliveries_readable"', 1, true) ~= nil, matter)
    t.is_true(matter:find('value = "false"', 1, true) ~= nil, matter)
  end,

  test_every_probe_failing_still_produces_an_unknown_report = function()
    local department = helper.department({
      observe = function()
        error("no snapshot")
      end,
      codex_runs = function()
        error("no codex status")
      end,
      commit_count = function()
        error("no git")
      end,
    })
    testing.run_fake(department, helper.tick())

    local matter = helper.front_matter(helper.only_report(department).text)
    t.eq(scalar(matter, "status"), "unknown")
    t.eq(scalar(matter, "confidence"), "low")
  end,

  -- ---- observe contract -----------------------------------------------------
  -- The durable root is never handed to observe as a string. The engine hashes
  -- FKST_DURABLE_ROOT exactly as the session was launched with it to find its
  -- live-observe socket, so a single appended slash degrades every call into a
  -- redb-lock error. Passing no argument makes the mutation impossible by
  -- construction, which is stronger than passing it carefully.
  test_observe_is_invoked_with_no_durable_root_argument = function()
    local department = helper.department()
    testing.run_fake(department, helper.tick())

    local observes = calls_of(department, "observe")
    t.eq(#observes, 1)
    t.eq(observes[1].argument_count, 0)
  end,

  -- ---- secret hygiene -------------------------------------------------------
  test_only_allowlisted_non_secret_environment_names_are_read = function()
    local department = helper.department()
    testing.run_fake(department, helper.tick())

    t.is_true(#department.seen_env > 0, "no environment was read at all")
    for _, name in ipairs(department.seen_env) do
      t.is_true(env_port.allowed[name] == true, "read a non-allowlisted env name: " .. name)
    end
    -- The credential-bearing session variables are absent from the allowlist itself,
    -- so no future edit can reach them without changing this list.
    t.eq(env_port.allowed.FKST_SESSION_CREDS_DIR, nil)
    t.eq(env_port.allowed.FKST_SESSION_DELIVERY_GRANTS, nil)
    t.eq(env_port.allowed.FKST_LLM_API_KEY, nil)
    t.eq(env_port.allowed.FKST_GITHUB_WRITE, nil)
  end,

  test_reading_a_denied_environment_name_is_refused = function()
    local ok, err = pcall(env_port.command_for, "FKST_SESSION_CREDS_DIR")
    t.eq(ok, false)
    t.is_true(tostring(err):find("fkst-health: env-name-denied", 1, true) ~= nil, tostring(err))
    t.is_true(env_port.command_for("FKST_SESSION_ID"):find("FKST_SESSION_ID", 1, true) ~= nil)
  end,

  -- The evidence context is what the judge reads. It carries counts, booleans, and
  -- engine-owned queue names -- never an environment value.
  test_no_environment_value_reaches_the_codex_evidence_context = function()
    local secret = "ghp-not-a-real-token-0123456789"
    local department = helper.department({
      env = {
        FKST_RUNTIME_ROOT = helper.runtime_root,
        FKST_SESSION_ID = helper.session,
        FKST_WORK_LABEL_NAMESPACE = helper.namespace,
        FKST_GITHUB_REPO = secret,
        FKST_SESSION_WORK_LABEL = secret,
      },
      github = {
        issue_search = function()
          return { stdout = "[]", stderr = "", exit_code = 0 }
        end,
      },
    })
    testing.run_fake(department, helper.tick())

    local context = department.fs.entries[helper.health_directory .. "/.fkst-health-context.md"]
    t.is_true(context ~= nil, "no evidence context was written")
    t.eq(context:find(secret, 1, true), nil)
    local spawns = calls_of(department, "codex")
    t.eq(spawns[1].opts.prompt:find(secret, 1, true), nil)
  end,

  -- ---- missing identity -----------------------------------------------------
  test_a_missing_session_id_writes_nothing_and_does_not_fail_the_tick = function()
    local department = helper.department({ env = { FKST_RUNTIME_ROOT = helper.runtime_root } })
    local result = testing.run_fake(department, helper.tick())

    t.eq(#helper.reports(department), 0)
    t.eq(#result.raises, 0)
    t.eq(#calls_of(department, "codex"), 0)
  end,

  -- ---- work items -----------------------------------------------------------
  test_open_work_items_are_relayed_and_an_empty_backlog_reports_idle = function()
    local department = helper.department({
      env = {
        FKST_RUNTIME_ROOT = helper.runtime_root,
        FKST_SESSION_ID = helper.session,
        FKST_GITHUB_REPO = "owner/repo",
        FKST_SESSION_WORK_LABEL = "fkst-dev",
      },
      github = {
        issue_search = function(repo, query)
          t.eq(repo, "owner/repo")
          t.is_true(query:find("fkst-dev", 1, true) ~= nil, query)
          return { stdout = '[{"number":812,"state":"OPEN"}]', stderr = "", exit_code = 0 }
        end,
      },
    })
    testing.run_fake(department, helper.tick())

    local matter = helper.front_matter(helper.only_report(department).text)
    t.eq(scalar(matter, "status"), "stalled")
    t.is_true(matter:find("[[work_items]]", 1, true) ~= nil, matter)
    t.is_true(matter:find("number = 812", 1, true) ~= nil, matter)
  end,

  test_a_closed_backlog_reports_idle = function()
    local department = helper.department({
      env = {
        FKST_RUNTIME_ROOT = helper.runtime_root,
        FKST_SESSION_ID = helper.session,
        FKST_GITHUB_REPO = "owner/repo",
        FKST_SESSION_WORK_LABEL = "fkst-dev",
      },
      github = {
        issue_search = function()
          return { stdout = "[]", stderr = "", exit_code = 0 }
        end,
      },
    })
    testing.run_fake(department, helper.tick())

    t.eq(scalar(helper.front_matter(helper.only_report(department).text), "status"), "idle")
  end,

  -- ---- window memory --------------------------------------------------------
  -- Consecutive quiet windows are what turn a possibly-slow codex turn into a
  -- confident stall. The counter lives in the engine's scratch cache, so this also
  -- proves a lost cache degrades to low confidence rather than to a wrong verdict.
  test_consecutive_quiet_windows_raise_stall_confidence = function()
    local cache = {}
    local first = helper.department({ cache = cache })
    testing.run_fake(first, helper.tick())
    t.eq(scalar(helper.front_matter(helper.only_report(first).text), "confidence"), "low")

    local second = helper.department({ cache = cache })
    testing.run_fake(second, helper.tick())
    t.eq(scalar(helper.front_matter(helper.only_report(second).text), "confidence"), "high")

    local forgotten = helper.department({ cache = {} })
    testing.run_fake(forgotten, helper.tick())
    t.eq(scalar(helper.front_matter(helper.only_report(forgotten).text), "confidence"), "low")
  end,

  -- ---- filename recognition -------------------------------------------------
  test_report_stamp_recognises_only_well_formed_report_names = function()
    t.eq(
      writer.report_stamp(helper.session .. "-health-agent-status-report-20260802-060000.md"),
      "20260802-060000"
    )
    for _, name in ipairs({
      "." .. helper.session .. "-health-agent-status-report-20260802-060000.md.partial",
      "notes.md",
      helper.session .. "-health-agent-status-report-2026080-060000.md",
      helper.session .. "-health-agent-status-report-20260802-06000.md",
      "-health-agent-status-report-20260802-060000.md",
      helper.session .. "-health-agent-status-report-20260802-060000.txt",
    }) do
      t.eq(writer.report_stamp(name), nil, name)
    end
  end,
}
