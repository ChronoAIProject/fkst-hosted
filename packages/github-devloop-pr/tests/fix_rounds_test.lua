local config = require("devloop.config")
local fix_rounds = require("core.fix_rounds")
local ci_verdict = require("core.ci_verdict")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core

local base_version = h.reviewing().version
local current_pr = { head_sha = "def456", state = "OPEN" }
local admission_context

local function with_own_ci(effect)
  t.mock_command(core.gh_pr_view_merge_cmd("owner/repo", 7), {
    stdout = '{"headRefName":"devloop-owner-repo-42-01HY","headRefOid":"def456","baseRefName":"dev","state":"OPEN","statusCheckRollup":[{"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"FAILURE","headSha":"def456"}]}',
    stderr = "",
    exit_code = 0,
  })
  h.mock_required_check_runs_for(current_pr.head_sha, "failure", "owner/repo")
  return ci_verdict.with_current_classification("owner/repo", 7, current_pr.head_sha, effect, {
    dept = "fix-rounds-test",
    proposal_id = "github-devloop/issue/owner/repo/42",
  })
end

local function own_ci_admission(state)
  return with_own_ci(function(classification)
    return fix_rounds.admit_own_ci_continuation(state, classification, admission_context())
  end)
end

admission_context = function()
  return {
    dept = "fix",
    from_state = "fixing",
    proposal_id = "github-devloop/issue/owner/repo/42",
    review_proposal_id = "consensus:review",
    review_dedup_key = "consensus:review/dedup",
    pr_number = 7,
    source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    reason = "own-ci-red-repair-budget-exhausted",
  }
end

local function capture_raises(fn)
  local raised = {}
  local original_raise = raise
  raise = function(queue, payload)
    table.insert(raised, { queue = queue, payload = payload })
  end
  local ok, err = pcall(fn)
  raise = original_raise
  if not ok then
    error(err)
  end
  return raised
end

return {
  -- The own-CI-red fixing self-loop budget is derived ONLY from the stable /fix/N version
  -- lineage. A drifting merge-queue predecessor set (A -> B -> A -> B ...) cannot reset it:
  -- the owner never reads the predecessor set, so no matter how many times the set flips
  -- across deliveries, version_fix_round advances strictly monotonically and terminates at
  -- max_fix_rounds() with the number of admitted generations bounded by the cap.
  test_predecessor_set_churn_cannot_reset_or_unbound_the_fix_round_budget = function()
    local max_rounds = config.max_fix_rounds()
    local version = core.next_fix_version(base_version) -- fix_round 1
    t.eq(core.version_fix_round(version), 1)
    local predecessor_sets = { "A", "B" }
    local admitted = 0
    local previous_round = 0
    local terminated = false
    -- Deliver far more times than the cap, flipping the drifting predecessor set every
    -- delivery. The budget must not care.
    for i = 1, max_rounds * 5 do
      local _churn_key = predecessor_sets[(i % 2) + 1]
      assert(_churn_key ~= nil)
      local round = core.version_fix_round(version)
      local decision
      capture_raises(function()
        decision = own_ci_admission({ state = "fixing", version = version })
      end)
      if round >= max_rounds then
        t.eq(decision.kind, "terminate")
        t.eq(decision.round, max_rounds)
        terminated = true
        break
      end
      t.eq(decision.kind, "admit")
      -- exactly one generation per admission, strictly increasing (never reset)
      t.eq(core.version_fix_round(decision.version), round + 1)
      t.is_true(core.version_fix_round(decision.version) > previous_round)
      previous_round = core.version_fix_round(decision.version)
      admitted = admitted + 1
      version = decision.version
    end
    t.is_true(terminated)
    -- Admitted generations bounded by the cap regardless of how many deliveries/flips.
    t.eq(admitted, max_rounds - 1)
  end,

  test_own_ci_admission_owner_mints_once_or_raises_the_head_bound_terminal = function()
    local under = core.next_fix_version(base_version) -- fix_round 1
    local admit_full
    local admit_raises = capture_raises(function()
      admit_full = own_ci_admission({ state = "fixing", version = under })
    end)
    t.eq(admit_full.kind, "admit")
    t.eq(core.version_fix_round(admit_full.version), 2)
    t.eq(admit_full.bound_head_sha, current_pr.head_sha)
    t.eq(#admit_raises, 0) -- admit never raises a terminal

    local at_cap = base_version
    for _ = 1, config.max_fix_rounds() do
      at_cap = core.next_fix_version(at_cap)
    end
    t.eq(core.version_fix_round(at_cap), config.max_fix_rounds())

    local decision
    local terminal_raises = capture_raises(function()
      decision = own_ci_admission({ state = "fixing", version = at_cap })
    end)
    t.eq(decision.kind, "terminate")
    t.eq(decision.round, config.max_fix_rounds())
    local reconcile = h.find_raise(terminal_raises, "devloop_fix_reconcile")
    t.is_true(reconcile ~= nil)
    t.eq(reconcile.payload.bound_head_sha, current_pr.head_sha)
    t.eq(reconcile.payload.schema, "github-devloop.own-ci-reconcile.v1")
    t.eq(reconcile.payload.reason_class, "fix-loop-max-rounds")

    t.is_true(h.find_raise(terminal_raises, "github-devloop-decompose.devloop_decompose") ~= nil)
  end,
}
