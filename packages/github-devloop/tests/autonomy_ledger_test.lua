local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function mock_check_runs(json)
  t.mock_command("gh api 'repos/owner/repo/commits/def456/check-runs'", {
    stdout = json,
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_valid_autonomous_merge_stays_pending_until_all_required_gates_pass = function()
    local gates = {
      human_touch = "pass",
      pre_merge_ci = "pass",
      evidence_manifest = "pending",
      post_merge_probe = "pending",
      no_revert_reopen = "pending",
      cost_budget = "pending",
    }
    t.eq(core.autonomy_valid_autonomous_merge(gates), "pending")

    gates.evidence_manifest = "pass"
    gates.post_merge_probe = "pass"
    gates.no_revert_reopen = "pass"
    gates.cost_budget = "pass"
    t.eq(core.autonomy_valid_autonomous_merge(gates), "true")

    gates.cost_budget = "fail"
    t.eq(core.autonomy_valid_autonomous_merge(gates), "false")
  end,

  test_autonomy_result_marker_recomputes_pending_predicate = function()
    local record = {
      proposal_id = "github-devloop/issue/owner/repo/42",
      repo = "owner/repo",
      issue_number = "42",
      pr_number = "7",
      version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/fix/2",
      head_sha = "def456",
      task_class = "L2",
      human_touch_count = 0,
      pre_merge_ci = "pass",
      rounds = 2,
      retry_count = 2,
      codex_calls = nil,
      gates = {
        human_touch = "pass",
        pre_merge_ci = "pass",
        evidence_manifest = "pending",
        post_merge_probe = "pending",
        no_revert_reopen = "pending",
        cost_budget = "pending",
      },
      valid_autonomous_merge = "true",
    }

    local marker = core.autonomy_result_marker(record)
    t.is_true(marker:find('valid_autonomous_merge="pending"', 1, true) ~= nil)
    t.is_true(marker:find('codex_calls="null"', 1, true) ~= nil)
    t.is_true(marker:find('post_merge_probe_green="pending"', 1, true) ~= nil)
    local fact = core.autonomy_result_fact({ marker }, record.proposal_id, record.pr_number, record.version, record.head_sha)
    t.eq(fact.valid_autonomous_merge, "pending")
    t.eq(fact.task_class, "L2")
    t.eq(fact.retry_count, 2)
    t.eq(fact.codex_calls, nil)
  end,

  test_post_merge_probe_gate_uses_existing_rollup_and_fails_closed = function()
    local green_gate = core.autonomy_post_merge_probe_gate({
      head_sha = "def456",
      status_check_rollup = {
        { status = "COMPLETED", conclusion = "SUCCESS" },
      },
    })
    t.eq(green_gate, "pass")

    local red_gate = core.autonomy_post_merge_probe_gate({
      head_sha = "def456",
      status_check_rollup = {
        { status = "COMPLETED", conclusion = "FAILURE" },
      },
    })
    t.eq(red_gate, "fail")

    mock_check_runs('{"total_count":0,"check_runs":[]}\n')
    local missing_gate = core.autonomy_post_merge_probe_gate({
      head_sha = "def456",
      status_check_rollup = {},
    }, { repo = "owner/repo" })
    t.eq(missing_gate, "fail")
  end,

  test_merged_marker_carries_canonical_autonomy_result_record = function()
    local record = {
      proposal_id = "github-devloop/issue/owner/repo/42",
      repo = "owner/repo",
      issue_number = "42",
      pr_number = "7",
      version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
      head_sha = "def456",
      task_class = "L2",
      human_touch_count = 0,
      rounds = 1,
      retry_count = 0,
      codex_calls = nil,
      gates = {
        human_touch = "pass",
        pre_merge_ci = "pass",
        evidence_manifest = "pending",
        post_merge_probe = "pending",
        no_revert_reopen = "pending",
        cost_budget = "pending",
      },
    }

    local marker = core.merged_marker(record.proposal_id, record.pr_number, record.version, record.head_sha, record)
    t.is_true(marker:find("fkst:github-devloop:merged:v1", 1, true) ~= nil)
    t.is_true(marker:find('autonomy_result="v1"', 1, true) ~= nil)
    t.is_true(marker:find('valid_autonomous_merge="pending"', 1, true) ~= nil)
    t.is_true(marker:find('gate_evidence_manifest="pending"', 1, true) ~= nil)
    local fact = core.merged_fact({ marker }, record.proposal_id, record.pr_number, record.version)
    t.eq(fact.autonomy_result.valid_autonomous_merge, "pending")
    t.eq(fact.autonomy_result.task_class, "L2")
  end,

  test_autonomy_result_fact_recomputes_predicate_from_parsed_gates = function()
    local record = {
      proposal_id = "github-devloop/issue/owner/repo/42",
      repo = "owner/repo",
      issue_number = "42",
      pr_number = "7",
      version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/fix/2",
      head_sha = "def456",
      task_class = "L2",
      human_touch_count = 0,
      rounds = 2,
      retry_count = 2,
      codex_calls = nil,
      gates = {
        human_touch = "pass",
        pre_merge_ci = "pass",
        evidence_manifest = "pending",
        post_merge_probe = "pending",
        no_revert_reopen = "pending",
        cost_budget = "pending",
      },
    }

    local marker = core.autonomy_result_marker(record):gsub(
      'valid_autonomous_merge="pending"',
      'valid_autonomous_merge="true"'
    )
    local fact = core.autonomy_result_fact({ marker }, record.proposal_id, record.pr_number, record.version, record.head_sha)
    t.eq(fact.valid_autonomous_merge, "pending")
  end,

  test_autonomy_auditor_rejects_forged_green_probe_without_matching_run = function()
    local record = {
      proposal_id = "github-devloop/issue/owner/repo/42",
      repo = "owner/repo",
      issue_number = "42",
      pr_number = "7",
      version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/fix/2",
      head_sha = "def456",
      task_class = "L2",
      human_touch_count = 0,
      pre_merge_ci = "pass",
      rounds = 2,
      retry_count = 2,
      codex_calls = nil,
      gates = {
        human_touch = "pass",
        pre_merge_ci = "pass",
        evidence_manifest = "pass",
        post_merge_probe = "pass",
        no_revert_reopen = "pass",
        cost_budget = "pass",
      },
    }

    mock_check_runs('{"total_count":0,"check_runs":[]}\n')
    local marker = core.autonomy_result_marker(record)
    local fact = core.autonomy_audited_result_fact(
      { marker },
      record.proposal_id,
      record.pr_number,
      record.version,
      record.head_sha,
      { repo = "owner/repo", merge_commit_sha = "def456" }
    )
    t.eq(fact.valid_autonomous_merge, "true")
    t.eq(fact.gates.post_merge_probe, "pass")
    t.eq(fact.audited_valid_autonomous_merge, "invalid_self_attested")
    t.eq(fact.audit_reason, "missing-status-rollup")
    t.eq(fact.audit_gates.post_merge_probe, "fail")
  end,

  test_task_class_uses_explicit_label_before_title_fallback = function()
    t.eq(core.autonomy_task_class({
      title = "fix scheduler regression",
      labels = { "fkst-avm:L4" },
    }), "L4")
    t.eq(core.autonomy_task_class({
      title = "docs: update readme",
      labels = {},
    }), "L0")
    t.eq(core.autonomy_task_class({
      title = "Add useful thing",
      labels = {},
    }), "unknown")
  end,
}
