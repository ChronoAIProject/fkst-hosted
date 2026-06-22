local h = require("tests.devloop_core_helpers")
local core = h.core
local error_facts = require("contract.error_facts")
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
      local rendered = type(cmd) == "table" and (cmd.cmd or table.concat(cmd.argv or {}, " ")) or cmd
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
  test_gh_exec_opts_preserves_argv_without_shell_controls = function()
    local spec = core.gh_exec_opts({ argv = { "gh", "issue", "list" }, timeout = 45 })
    t.eq(spec.argv[1], "gh")
    t.eq(spec.argv[2], "issue")
    t.eq(spec.argv[3], "list")
    t.eq(spec.timeout, 45)
    t.is_nil(spec.cmd)
    t.is_nil(spec.rate_pool)
  end,
  test_github_high_risk_paths_cover_ci_auth_dependency_and_scheduler_surfaces = function()
    local high = core.github_high_risk_paths({
      ".github/workflows/ci.yml",
      "Cargo.lock",
      "scripts/run.sh",
      "packages/github-devloop/core.lua",
    })
    t.eq(#high, 3)
    t.eq(high[1], ".github/workflows/ci.yml")
    t.eq(high[2], "Cargo.lock")
    t.eq(high[3], "scripts/run.sh")
  end,
  test_core_shared_surface_keeps_two_copy_helpers_local = function()
    t.is_nil(core.age_minutes)
    t.is_nil(core.valid_round)
  end,
  test_parse_name_only_paths_trims_deduplicates_and_sorts = function()
    local paths = core.parse_name_only_paths("  b.lua\r\na.lua\n\n b.lua \r  c.lua  \n")
    t.eq(#paths, 3)
    t.eq(paths[1], "a.lua")
    t.eq(paths[2], "b.lua")
    t.eq(paths[3], "c.lua")
  end,
  test_core_shared_judgment_worktree_reads_runtime_root_and_mkdirs = function()
    local worktree = core.judgment_worktree_path("/tmp/fkst-runtime\n", "review-meta", "dedup/key")
    t.eq(core.mkdir_p_cmd(worktree), "mkdir -p '" .. worktree .. "'")
    t.is_nil(core.mkdir_p_cmd(worktree):find("chmod", 1, true))
    t.mock_command(core.read_runtime_root_cmd(), {
      stdout = "/tmp/fkst-runtime\n",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.mkdir_p_cmd(worktree), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })

    local actual = core.judgment_worktree("review-meta", "dedup/key")

    t.eq(actual, worktree)
    local saw_mkdir = false
    for _, call in ipairs(t.command_calls()) do
      if call.rendered == core.mkdir_p_cmd(worktree) then
        saw_mkdir = true
      end
    end
    t.eq(saw_mkdir, true)
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
    local fields = error_facts.error_fact_fields(
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
    t.eq(fields[2], "fingerprint=" .. error_facts.error_fingerprint(
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
    local fields = error_facts.error_fact_fields("codex-failed", "devloop_ready", "implement", "codex failed", {})

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
