local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core

local function pr(extra)
  local value = {
    state = "OPEN",
    head_sha = "def456",
    head_ref_name = "integration/dev",
    base_ref_name = "dev",
    head_repository = "owner/repo",
    is_cross_repository = false,
    mergeable = "MERGEABLE",
    merge_state_status = "CLEAN",
    status_check_rollup = {
      { name = "ci", state = "COMPLETED", conclusion = "SUCCESS" },
    },
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local expected = {
  repo = "owner/repo",
  head_sha = "def456",
  head_branch = "integration/dev",
  base_branch = "dev",
}

local function mock_check_runs(json)
  t.mock_command("gh api 'repos/owner/repo/commits/def456/check-runs'", {
    stdout = json,
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_pr_identity_matches_true = function()
    local ok, reason = core.pr_identity_matches(pr(), expected)
    t.eq(ok, true)
    t.eq(reason, "pr-ok")
  end,

  test_pr_identity_matches_false_cases = function()
    local ok, reason = core.pr_identity_matches(pr({ state = "MERGED" }), expected)
    t.eq(ok, false)
    t.eq(reason, "pr-not-open")

    ok, reason = core.pr_identity_matches(pr({ head_sha = "aaaa1111" }), expected)
    t.eq(ok, false)
    t.eq(reason, "head-sha-mismatch")

    ok, reason = core.pr_identity_matches(pr({ head_ref_name = "feature/x" }), expected)
    t.eq(ok, false)
    t.eq(reason, "head-branch-mismatch")

    ok, reason = core.pr_identity_matches(pr({ base_ref_name = "main" }), expected)
    t.eq(ok, false)
    t.eq(reason, "base-branch-mismatch")

    ok, reason = core.pr_identity_matches(pr({ is_cross_repository = true }), expected)
    t.eq(ok, false)
    t.eq(reason, "foreign-head-repository")
  end,

  test_evaluate_ci_merge_gate_true = function()
    local ok, reason = core.evaluate_ci_merge_gate(pr())
    t.eq(ok, true)
    t.eq(reason, "merge-gate-ok")
  end,

  test_evaluate_ci_merge_gate_false_cases = function()
    local ok, reason = core.evaluate_ci_merge_gate(pr({
      status_check_rollup = {
        { name = "ci", state = "COMPLETED", conclusion = "FAILURE" },
      },
    }))
    t.eq(ok, false)
    t.eq(reason, "rollup-red")

    ok, reason = core.evaluate_ci_merge_gate(pr({
      status_check_rollup = {
        { name = "ci", state = "COMPLETED", conclusion = "NEUTRAL" },
      },
    }))
    t.eq(ok, false)
    t.eq(reason, "rollup-red")

    ok, reason = core.evaluate_ci_merge_gate(pr({ mergeable = "CONFLICTING" }))
    t.eq(ok, false)
    t.eq(reason, "mergeable-conflicting")
  end,

  test_empty_rollup_falls_back_to_required_commit_check_run_green = function()
    mock_check_runs('{"total_count":2,"check_runs":[{"name":"unrelated","status":"completed","conclusion":"success"},{"name":"test","status":"completed","conclusion":"success"}]}\n')
    local ok, reason = core.evaluate_ci_status_gate(pr({ status_check_rollup = {} }), {
      repo = "owner/repo",
      proposal_id = "github-devloop/issue/owner/repo/42",
    })
    t.eq(ok, true)
    t.eq(reason, "rollup-green")
  end,

  test_empty_rollup_fallback_red_required_commit_check_run = function()
    mock_check_runs('{"total_count":1,"check_runs":[{"name":"test","status":"completed","conclusion":"failure"}]}\n')
    local ok, reason = core.evaluate_ci_status_gate(pr({ status_check_rollup = {} }), {
      repo = "owner/repo",
    })
    t.eq(ok, false)
    t.eq(reason, "rollup-red")
  end,

  test_empty_rollup_fallback_pending_required_commit_check_run = function()
    mock_check_runs('{"total_count":1,"check_runs":[{"name":"test","status":"in_progress","conclusion":null}]}\n')
    local ok, reason = core.evaluate_ci_status_gate(pr({ status_check_rollup = {} }), {
      repo = "owner/repo",
    })
    t.eq(ok, false)
    t.eq(reason, "rollup-pending")
  end,

  test_empty_rollup_fallback_absent_commit_check_runs_stays_missing = function()
    mock_check_runs('{"total_count":0,"check_runs":[]}\n')
    local ok, reason = core.evaluate_ci_status_gate(pr({ status_check_rollup = {} }), {
      repo = "owner/repo",
    })
    t.eq(ok, false)
    t.eq(reason, "missing-status-rollup")
  end,

  test_empty_rollup_fallback_missing_required_check_stays_missing = function()
    mock_check_runs('{"total_count":1,"check_runs":[{"name":"docs","status":"completed","conclusion":"success"}]}\n')
    local ok, reason = core.evaluate_ci_status_gate(pr({ status_check_rollup = {} }), {
      repo = "owner/repo",
    })
    t.eq(ok, false)
    t.eq(reason, "missing-status-rollup")
  end,

  test_missing_status_dispatch_eligibility_uses_first_observed_time = function()
    local eligible, reason, age = core.ci_missing_status_dispatch_eligible(pr({
      status_check_rollup = {},
    }), 600, 240, 300)
    t.eq(eligible, true)
    t.eq(reason, "missing-status-rollup")
    t.eq(age, 360)

    eligible, reason = core.ci_missing_status_dispatch_eligible(pr({
      status_check_rollup = {},
      updated_at = "2026-06-03T02:00:00Z",
    }), 600, 420, 300)
    t.eq(eligible, false)
    t.eq(reason, "missing-status-grace")

    eligible, reason = core.ci_missing_status_dispatch_eligible(pr({
      status_check_rollup = {
        { name = "ci", state = "IN_PROGRESS", conclusion = "" },
      },
    }), 600, 240, 300)
    t.eq(eligible, false)
    t.eq(reason, "rollup-pending")
  end,
}
