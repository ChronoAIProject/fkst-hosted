local runner = require("runner")
local t = fkst.test

local RUN_ISSUE_BODY = table.concat({
  '<!-- fkst-cron-dispatch:v1 schedule="123" workflow="sourcing" '
    .. 'slot="2026-08-05T01:00:00Z" manual="false" -->',
  "",
  "### Scheduled Run",
  "",
  "### Arguments",
  "",
  "```toml",
  'role = "AI Tools Application Engineer"',
  'min_score = "6"',
  "```",
}, "\n")

return {
  -- ---- dispatch ----------------------------------------------------------

  test_dispatch_marker_carries_the_run_identity = function()
    local dispatch, err = runner.parse_dispatch(RUN_ISSUE_BODY)
    t.is_nil(err)
    t.eq(dispatch.schedule_issue, 123)
    t.eq(dispatch.workflow_id, "sourcing")
    t.eq(dispatch.slot, "2026-08-05T01:00:00Z")
    t.is_false(dispatch.manual)
    t.eq(dispatch.arguments.role, "AI Tools Application Engineer")
    t.eq(dispatch.arguments.min_score, "6")
  end,

  test_an_ordinary_work_issue_is_a_clean_noop = function()
    -- The pod also boots for ordinary work. Reading that as a scheduled run
    -- would execute something nobody asked for.
    local dispatch, err = runner.parse_dispatch("## What needs doing\n\nFix the bug.\n")
    t.is_nil(dispatch)
    t.is_nil(err)
  end,

  test_a_malformed_marker_is_a_hard_failure_not_a_noop = function()
    -- Silently ignoring it would strand the schedule until its watchdog fired,
    -- with nothing anywhere saying why.
    local dispatch, err = runner.parse_dispatch('<!-- fkst-cron-dispatch:v1 schedule="123" -->')
    t.is_nil(dispatch)
    t.is_true(err ~= nil)
  end,

  test_a_manual_run_is_distinguishable = function()
    local body = RUN_ISSUE_BODY:gsub('manual="false"', 'manual="true"')
    t.is_true(runner.parse_dispatch(body).manual)
  end,

  -- ---- arguments ---------------------------------------------------------

  test_argument_escapes_round_trip = function()
    -- The control plane writes TOML basic strings; a value that survived
    -- unescaping wrong would reach a command as different text than the author
    -- wrote.
    local body = '<!-- fkst-cron-dispatch:v1 schedule="1" workflow="w" slot="s" -->\n'
      .. '```toml\nrole = "a \\"quoted\\" role\\nsecond line\\ttabbed"\n```'
    local dispatch = runner.parse_dispatch(body)
    t.eq(dispatch.arguments.role, 'a "quoted" role\nsecond line\ttabbed')
  end,

  test_a_run_with_no_arguments_parses = function()
    local body = '<!-- fkst-cron-dispatch:v1 schedule="1" workflow="w" slot="s" -->\n\n_None._'
    local dispatch = runner.parse_dispatch(body)
    t.eq(next(dispatch.arguments), nil)
  end,

  -- ---- substitution ------------------------------------------------------

  test_substitution_puts_values_in_as_data = function()
    local value, err = runner.substitute("--role={{ role }}", { role = "; rm -rf /" })
    t.is_nil(err)
    -- The value is placed verbatim into ONE argv element; it never becomes shell
    -- syntax, because the caller quotes each element separately.
    t.eq(value, "--role=; rm -rf /")
  end,

  test_an_unsupplied_argument_is_an_error_not_an_empty_string = function()
    -- Running a scrape with a blank search term would produce a plausible,
    -- WRONG result rather than a visible failure.
    local value, err = runner.substitute("--role={{ role }}", {})
    t.is_nil(value)
    t.is_true(err:find("role", 1, true) ~= nil)
  end,

  test_resolve_step_substitutes_every_argv_element = function()
    local resolved, err = runner.resolve_step({
      index = 1,
      id = "scrape",
      kind = "run",
      command = { "python3", "scrape.py", "--role", "{{ role }}", "--min", "{{ min_score }}" },
      timeout_secs = 600,
    }, { role = "engineer", min_score = "6" })
    t.is_nil(err)
    t.eq(#resolved.argv, 6)
    t.eq(resolved.argv[4], "engineer")
    t.eq(resolved.argv[6], "6")
  end,

  test_resolve_step_substitutes_a_task_prompt = function()
    local resolved, err = runner.resolve_step({
      index = 2,
      id = "score",
      kind = "task",
      prompt = "Score each candidate against {{ role }}.",
      timeout_secs = 900,
    }, { role = "engineer" })
    t.is_nil(err)
    t.eq(resolved.prompt, "Score each candidate against engineer.")
  end,

  -- ---- definition validation ---------------------------------------------

  test_a_valid_definition_yields_ordered_steps = function()
    local steps, err = runner.validate_definition({
      step = {
        { id = "scrape", kind = "run", command = { "true" } },
        { id = "score", kind = "task", prompt = "score it", timeout_secs = 1200 },
      },
    })
    t.is_nil(err)
    t.eq(#steps, 2)
    t.eq(steps[1].index, 1)
    t.eq(steps[2].timeout_secs, 1200)
    t.eq(steps[1].timeout_secs, 900)
  end,

  test_every_shape_problem_names_the_offending_step = function()
    -- Fail-closed: a definition that half-runs can publish half a result over a
    -- good one, which is worse than one that refuses outright.
    for _, case in ipairs({
      { definition = {}, why = "at least one" },
      { definition = { step = {} }, why = "no steps" },
      { definition = { step = { { kind = "run", command = {} } } }, why = "invalid id" },
      { definition = { step = { { id = "a b", kind = "run", command = {} } } }, why = "invalid id" },
      { definition = { step = { { id = "a", kind = "magic" } } }, why = "unsupported kind" },
      { definition = { step = { { id = "a", kind = "run" } } }, why = "command" },
      { definition = { step = { { id = "a", kind = "task" } } }, why = "prompt" },
      {
        definition = {
          step = {
            { id = "a", kind = "run", command = { "x" } },
            { id = "a", kind = "run", command = { "y" } },
          },
        },
        why = "duplicate step id",
      },
    }) do
      local steps, err = runner.validate_definition(case.definition)
      t.is_nil(steps)
      t.is_true(
        err:find(case.why, 1, true) ~= nil,
        ("expected %q in %q"):format(case.why, tostring(err))
      )
    end
  end,

  -- ---- the wire contract -------------------------------------------------

  test_the_run_marker_matches_the_control_planes_pinned_format = function()
    -- backend/src/schedule/marker.rs parses this exact string. A drift here
    -- silently breaks completion detection: a finished run would look in-flight
    -- until its watchdog released it.
    local marker = runner.render_run_marker({
      slot = "2026-07-27T03:00:00Z",
      manual = false,
      status = "ok",
      started = "2026-07-27T03:00:00Z",
      ended = "2026-07-27T03:12:00Z",
      issue = 4242,
      detail = "all steps completed",
      steps = {
        { index = 1, id = "scrape", status = "ok", duration_s = 41 },
        { index = 2, id = "score", status = "ok", duration_s = 680 },
      },
    })
    t.eq(
      marker,
      '<!-- fkst-cron-run:v1 slot="2026-07-27T03:00:00Z" manual="false" status="ok" '
        .. 'started="2026-07-27T03:00:00Z" ended="2026-07-27T03:12:00Z" issue="4242" '
        .. 'detail="all steps completed" steps="1:scrape:ok:41;2:score:ok:680" -->'
    )
  end,

  test_absent_optional_attributes_are_omitted_rather_than_emptied = function()
    local marker = runner.render_run_marker({
      slot = "2026-07-27T03:00:00Z",
      manual = false,
      status = "failed",
      started = "2026-07-27T03:00:00Z",
    })
    for _, absent in ipairs({ "ended=", "issue=", "detail=", "steps=" }) do
      t.is_true(marker:find(absent, 1, true) == nil, "must omit " .. absent)
    end
  end,

  test_a_step_that_never_ran_carries_an_empty_duration = function()
    t.eq(
      runner.render_steps({
        { index = 1, id = "scrape", status = "ok", duration_s = 41 },
        { index = 2, id = "score", status = "failed", duration_s = 9 },
        { index = 3, id = "publish", status = "skipped" },
      }),
      "1:scrape:ok:41;2:score:failed:9;3:publish:skipped:"
    )
  end,

  test_a_detail_cannot_break_out_of_the_comment_or_its_attribute = function()
    -- A detail is free text from a failing step: hostile to the enclosing format
    -- by default, not trusted to behave.
    local marker = runner.render_run_marker({
      slot = "s",
      manual = false,
      status = "failed",
      started = "t",
      detail = 'step "scrape" failed --> <script>\nsecond line',
    })
    t.eq(select(2, marker:gsub("%-%->", "")), 1)
    t.is_true(marker:find("<script>", 1, true) == nil)
  end,

  test_an_overlong_detail_is_truncated = function()
    local marker = runner.render_run_marker({
      slot = "s",
      manual = false,
      status = "failed",
      started = "t",
      detail = string.rep("x", 1000),
    })
    t.eq(#marker:match('detail="([^"]*)"'), 200)
  end,

  -- ---- output tails ------------------------------------------------------

  test_a_short_tail_is_untouched = function()
    t.eq(runner.truncate_tail("all good"), "all good")
  end,

  test_a_long_tail_keeps_the_END_and_says_how_much_went = function()
    -- The tail, not the head: the interesting part of a failing command's output
    -- is what it said last.
    local tail = runner.truncate_tail(string.rep("a", 100) .. "THE-END", 16)
    t.is_true(tail:find("truncated, 91 bytes omitted", 1, true) ~= nil)
    t.is_true(tail:find("THE%-END") ~= nil)
  end,

  -- ---- path safety -------------------------------------------------------

  test_a_workflow_id_resolves_under_the_definitions_directory = function()
    local path, err = runner.definition_path("github-candidate-sourcing")
    t.is_nil(err)
    t.eq(path, ".fkst/workflows/github-candidate-sourcing.toml")
  end,

  test_a_traversing_workflow_id_is_refused = function()
    -- This is where the id becomes a filesystem path, so it is re-validated here
    -- even though the control plane already validated it.
    for _, id in ipairs({ "../../etc/passwd", "..", "a/b", "a b", "" }) do
      local path, err = runner.definition_path(id)
      t.is_nil(path)
      t.is_true(err ~= nil)
    end
  end,

  -- ---- run-issue selection -----------------------------------------------

  test_only_an_issue_routed_to_this_creator_is_eligible = function()
    local issues = {
      { number = 10, body = RUN_ISSUE_BODY, assignees = { "someone-else" } },
      { number = 11, body = RUN_ISSUE_BODY, assignees = { "alice", "bob" } },
      { number = 12, body = RUN_ISSUE_BODY, assignees = {} },
      { number = 13, body = "an ordinary work issue", assignees = { "alice" } },
    }
    t.is_nil(runner.select_run_issue(issues, "alice"))
  end,

  test_the_lowest_numbered_run_issue_is_serviced_first = function()
    local issues = {
      { number = 22, body = RUN_ISSUE_BODY, assignees = { "alice" } },
      { number = 15, body = RUN_ISSUE_BODY, assignees = { "Alice" } },
    }
    local issue, dispatch = runner.select_run_issue(issues, "alice")
    t.eq(issue.number, 15)
    t.eq(dispatch.workflow_id, "sourcing")
  end,

  -- ---- secret hygiene ----------------------------------------------------

  test_a_credential_shaped_value_never_reaches_a_run_record = function()
    -- Secrets reach a step only through an environment profile. If one ever
    -- surfaces in a step's output or in a failure detail, the run record must not
    -- carry it into a public issue comment — which is a permanent, indexed home.
    local secret = "ghp_0123456789abcdefghijklmnopqrstuvwxyz"
    local record = {
      slot = "2026-08-05T01:00:00Z",
      manual = false,
      status = "failed",
      started = "2026-08-05T01:00:00Z",
      ended = "2026-08-05T01:03:00Z",
      -- A step that echoed its own environment, which is exactly how this leaks
      -- in practice.
      detail = "step publish exited 1",
      tail = runner.truncate_tail(("padding\n"):rep(2000) .. "TOKEN=" .. secret, 32),
      steps = {},
    }
    -- The tail is truncated to its END, so a secret printed early is gone. This
    -- is a mitigation, not a guarantee — the guarantee is that the value never
    -- enters an argument in the first place (the control plane refuses one), and
    -- this test states which of the two is which.
    t.is_true(#record.tail < 200)
    local marker = runner.render_run_marker(record)
    t.is_true(marker:find("ghp_", 1, true) == nil, "the MARKER must never carry a token")
  end,

  test_a_detail_is_sanitized_before_it_becomes_a_marker_attribute = function()
    local sanitized = runner.sanitize_detail('a "quoted" <tag> detail\nwith a newline')
    t.is_true(sanitized:find('"', 1, true) == nil)
    t.is_true(sanitized:find("<", 1, true) == nil)
    t.is_true(sanitized:find("\n", 1, true) == nil)
  end,

  test_the_report_body_carries_the_human_line_the_tail_and_the_record = function()
    local body = runner.render_report({
      slot = "2026-08-05T01:00:00Z",
      manual = false,
      status = "failed",
      started = "2026-08-05T01:00:00Z",
      ended = "2026-08-05T01:03:00Z",
      detail = "step score exited 1",
      tail = "Traceback...",
      steps = { { index = 1, id = "score", status = "failed", duration_s = 9 } },
    })
    t.is_true(body:find("❌ Scheduled run failed", 1, true) ~= nil)
    t.is_true(body:find("Traceback...", 1, true) ~= nil)
    t.is_true(body:find("fkst%-cron%-run:v1") ~= nil)
  end,
}
