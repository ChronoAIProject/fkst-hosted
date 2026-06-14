local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t
local source_ref = h.source_ref
local issue = h.issue

return {
  test_devloop_config_defaults_and_validation = function()
    local responses = {
      ['printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"'] = { stdout = "", exit_code = 0 },
      ["git rev-parse --abbrev-ref HEAD"] = { stdout = "dev\n", exit_code = 0 },
      ['printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"'] = { stdout = "", exit_code = 0 },
      ['printf %s "$FKST_DEVLOOP_ROLLUP_MERGE"'] = { stdout = "", exit_code = 0 },
      ['printf %s "$FKST_DEVLOOP_TEST_COMMAND"'] = { stdout = "", exit_code = 0 },
      ['printf %s "$FKST_DEVLOOP_INTAKE_PROBE_PROOF"'] = { stdout = "", exit_code = 0 },
      ['printf %s "$FKST_GITHUB_REPO"'] = { stdout = "owner/repo", exit_code = 0 },
      ['printf %s "$FKST_GITHUB_BOT_LOGIN"'] = { stdout = "fkst-test-bot", exit_code = 0 },
      ['printf %s "$FKST_GITHUB_WRITE"'] = { stdout = "", exit_code = 0 },
    }
    local function exec(cmd)
      local rendered = type(cmd) == "table" and cmd.cmd or cmd
      return responses[rendered] or { stdout = "", stderr = "unexpected " .. tostring(rendered), exit_code = 1 }
    end
    local config = core.devloop_config(exec)
    t.eq(config.repo, "owner/repo")
    t.eq(config.bot_login, "fkst-test-bot")
    t.eq(config.write_mode, "dry-run")
    t.eq(config.upstream_branch, "dev")
    t.eq(config.integration_branch, "dev")
    t.eq(config.rollup_merge, "auto")
    t.eq(core.test_command(exec), "scripts/run.sh test")
    t.eq(core.intake_probe_gate(exec).enabled, false)

    t.eq(core.env_present_command("GH_TOKEN"), 'if [ -n "${GH_TOKEN:-}" ]; then printf present; fi')
    responses[core.env_present_command("GH_TOKEN")] = { stdout = "present", exit_code = 0 }
    responses[core.env_present_command("GITHUB_TOKEN")] = { stdout = "", exit_code = 0 }
    t.eq(core.env_present("GH_TOKEN", exec), true)
    t.eq(core.env_present("GITHUB_TOKEN", exec), false)
    t.raises(function()
      core.read_env_command("GH_TOKEN")
    end)

    responses['printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"'] = { stdout = "main", exit_code = 0 }
    responses['printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"'] = { stdout = "integration/dev", exit_code = 0 }
    responses['printf %s "$FKST_DEVLOOP_ROLLUP_MERGE"'] = { stdout = "manual", exit_code = 0 }
    responses['printf %s "$FKST_DEVLOOP_TEST_COMMAND"'] = { stdout = "cargo build && cargo test", exit_code = 0 }
    responses['printf %s "$FKST_DEVLOOP_INTAKE_PROBE_PROOF"'] = { stdout = "event-fast-path-insufficient", exit_code = 0 }
    responses['printf %s "$FKST_GITHUB_WRITE"'] = { stdout = "1", exit_code = 0 }
    config = core.devloop_config(exec)
    t.eq(config.write_mode, "real")
    t.eq(config.upstream_branch, "main")
    t.eq(config.integration_branch, "integration/dev")
    t.eq(config.rollup_merge, "manual")
    t.eq(core.test_command(exec), "cargo build && cargo test")
    t.eq(core.intake_probe_gate(exec).enabled, true)

    responses['printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"'] = { stdout = "../bad", exit_code = 0 }
    t.raises(function()
      core.branch_config(exec)
    end)
    responses['printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"'] = { stdout = "integration/dev", exit_code = 0 }
    responses['printf %s "$FKST_DEVLOOP_ROLLUP_MERGE"'] = { stdout = "sometimes", exit_code = 0 }
    t.raises(function()
      core.devloop_config(exec)
    end)
    responses['printf %s "$FKST_DEVLOOP_INTAKE_PROBE_PROOF"'] = { stdout = "enabled", exit_code = 0 }
    t.raises(function()
      core.intake_probe_gate(exec)
    end)
  end,
  test_gh_exec_opts_uses_shared_rate_pool = function()
    local spec = core.gh_exec_opts({ cmd = "gh issue list", timeout = 45 })
    t.eq(spec.cmd, "gh issue list")
    t.eq(spec.timeout, 45)
    t.eq(spec.rate_pool.name, "gh")
    t.eq(spec.rate_pool.burst, nil)
    t.eq(spec.rate_pool.refill_per_hour, nil)
  end,
  test_opt_in_detection = function()
    t.eq(core.is_opted_in({ "fkst-dev:enabled" }), true)
    t.eq(core.is_opted_in({ "bug" }), false)
    t.eq(core.is_opted_in({ "fkst-dev:enabled", "fkst-dev:thinking" }), true)
    t.eq(core.is_opted_in({ "fkst-dev:enabled", "fkst-dev:ready" }), true)
    t.eq(core.is_opted_in({ "fkst-dev:enabled", "fkst-dev:impl-failed" }), true)
    t.eq(core.is_opted_in({ "fkst-dev:enabled", "fkst-dev:blocked" }), true)
  end,
  test_proposal_id_round_trip = function()
    local id = core.proposal_id("owner/repo", 42)
    t.eq(id, "github-devloop/issue/owner/repo/42")
    local repo, issue_number = core.parse_proposal_id(id)
    t.eq(repo, "owner/repo")
    t.eq(issue_number, "42")
    t.eq(core.issue_ref_round_trips("owner/repo", 42), true)
    t.is_nil(core.parse_proposal_id("autochrono/issue/owner/repo/42"))
  end,
  test_error_fact_fields_include_available_delivery_context = function()
    local fields = core.error_fact_fields(
      "codex-failed",
      "devloop_ready",
      "implement",
      "codex failed at 2026-06-10T01:02:03Z on abcdef1234567890 in /tmp/fkst-a",
      {
        source_ref = source_ref(),
        attempt = 4,
        terminal = false,
      }
    )

    t.eq(fields[1], "error_class=codex-failed")
    t.eq(fields[2], "fingerprint=" .. core.error_fingerprint(
      "codex-failed",
      "devloop_ready",
      "implement",
      "codex failed at 2027-07-11T09:08:07Z on fedcba0987654321 in /tmp/fkst-b"
    ))
    t.eq(fields[3], "source_ref=external:owner/repo#issue/42")
    t.eq(fields[4], "attempt=4")
    t.eq(fields[5], "terminal=false")
  end,
  test_error_fact_fields_omit_unavailable_delivery_context = function()
    local fields = core.error_fact_fields("codex-failed", "devloop_ready", "implement", "codex failed", {})

    t.eq(#fields, 2)
    t.eq(fields[1], "error_class=codex-failed")
    t.is_true(fields[2]:find("^fingerprint=fp%-") ~= nil)
  end,
  test_log_codex_result_emits_structured_failure_line = function()
    local captured = {}
    local old_log = log
    log = {
      error = function(message)
        table.insert(captured, tostring(message))
      end,
    }

    core.log_codex_result(
      "implement",
      "github-devloop/issue/owner/repo/42",
      "implement",
      { exit_code = 1 },
      nil,
      "codex failed",
      {
        queue = "devloop_ready",
        source_ref = source_ref(),
        terminal = false,
      }
    )
    log = old_log

    t.eq(#captured, 1)
    t.is_true(captured[1]:find("github-devloop dept=implement", 1, true) ~= nil)
    t.is_true(captured[1]:find("tag=CODEX", 1, true) ~= nil)
    t.is_true(captured[1]:find("error_class=codex-failed", 1, true) ~= nil)
    t.is_true(captured[1]:find("fingerprint=", 1, true) ~= nil)
    t.is_true(captured[1]:find("source_ref=external:owner/repo#issue/42", 1, true) ~= nil)
    t.is_true(captured[1]:find("terminal=false", 1, true) ~= nil)
  end,
  test_wrapped_pipeline_failure_logs_delivery_error_fact_and_rethrows = function()
    local captured = {}
    local old_log = log
    log = {
      error = function(message)
        table.insert(captured, tostring(message))
      end,
    }

    local wrapped = core.wrap_pipeline_failure("implement", function(_event)
      error("github-devloop: gh-issue-view-failed: bad sha abcdef1234567890 at 2026-06-10T01:02:03Z /tmp/fkst-a")
    end)
    local ok, err = pcall(function()
      wrapped({
        queue = "devloop_ready",
        attempt = 4,
        terminal = false,
        payload = {
          proposal_id = "github-devloop/issue/owner/repo/42",
          source_ref = source_ref(),
        },
      })
    end)

    log = old_log
    t.eq(ok, false)
    t.is_true(tostring(err):find("gh-issue-view-failed", 1, true) ~= nil)
    t.eq(#captured, 1)
    t.is_true(captured[1]:find("github-devloop dept=implement proposal_id=github-devloop/issue/owner/repo/42 tag=FAILURE", 1, true) ~= nil)
    t.is_true(captured[1]:find("error_class=gh-issue-view-failed", 1, true) ~= nil)
    t.is_true(captured[1]:find("fingerprint=", 1, true) ~= nil)
    t.is_true(captured[1]:find("source_ref=external:owner/repo#issue/42", 1, true) ~= nil)
    t.is_true(captured[1]:find("attempt=4", 1, true) ~= nil)
    t.is_nil(captured[1]:find("terminal=", 1, true))
    t.is_true(captured[1]:find("queue=devloop_ready", 1, true) ~= nil)
  end,
  test_error_class_from_message_prefers_inner_codex_failure = function()
    t.eq(
      core.error_class_from_message("github-devloop: fix codex failed: bad sha abcdef1234567890"),
      "codex-failed"
    )
    t.eq(
      core.error_class_from_message("github-devloop: intake codex failed: timed out"),
      "codex-failed"
    )
  end,
  test_build_proposal = function()
    local proposal = core.build_proposal(issue())
    t.eq(proposal.schema, "consensus.proposal.v1")
    t.eq(proposal.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(proposal.title, "Implement decision recorder")
    t.is_true(#proposal.body < 256)
    t.is_true(proposal.body:find("GitHub issue", 1, true) ~= nil)
    t.is_nil(proposal.body:find("Issue body", 1, true))
    t.is_nil(proposal.content_fetch)
    t.eq(proposal.dedup_key, "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z")
    t.eq(proposal.source_ref.ref, "owner/repo#issue/42")
    t.eq(core.validate_proposal(proposal), true)
  end,
}
