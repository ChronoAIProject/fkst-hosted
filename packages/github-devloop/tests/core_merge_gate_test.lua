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
}
