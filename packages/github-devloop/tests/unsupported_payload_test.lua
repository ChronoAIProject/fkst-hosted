local t = fkst.test
local core = require("core")

local cases = {
  {
    dept = "loop",
    path = "departments/loop/main.lua",
    queue = "consensus.consensus_converge",
  },
  {
    dept = "review_loop",
    path = "departments/review_loop/main.lua",
    queue = "consensus.consensus_converge",
  },
  {
    dept = "review_result",
    path = "departments/review_result/main.lua",
    queue = "consensus.consensus_reached",
  },
  {
    dept = "review_meta",
    path = "departments/review_meta/main.lua",
    queue = "devloop_review_meta",
  },
  {
    dept = "decompose",
    path = "departments/decompose/main.lua",
    queue = "devloop_decompose",
  },
  {
    dept = "review_pr",
    path = "departments/review_pr/main.lua",
    queue = "devloop_reviewing",
  },
  {
    dept = "implement",
    path = "departments/implement/main.lua",
    queue = "devloop_ready",
  },
  {
    dept = "consensus_result",
    path = "departments/consensus_result/main.lua",
    queue = "consensus.consensus_reached",
  },
  {
    dept = "open_pr",
    path = "departments/open_pr/main.lua",
    queue = "devloop_open_pr",
  },
  {
    dept = "fix",
    path = "departments/fix/main.lua",
    queue = "devloop_fixing",
  },
  {
    dept = "observe_pr",
    path = "departments/observe_pr/main.lua",
    queue = "github-proxy.github_entity_changed",
  },
  {
    dept = "intake_judge",
    path = "departments/intake_judge/main.lua",
    queue = "devloop_intake_candidate",
  },
  {
    dept = "merge",
    path = "departments/merge/main.lua",
    queue = "devloop_merge_ready",
  },
  {
    dept = "observe_issue",
    path = "departments/observe_issue/main.lua",
    queue = "github-proxy.github_entity_changed",
  },
  {
    dept = "sync_conflict",
    path = "departments/sync_conflict/main.lua",
    queue = "devloop_sync_conflict",
  },
  {
    dept = "reconcile",
    path = "departments/reconcile/main.lua",
    queue = "devloop_reconcile",
  },
  {
    dept = "review_reconcile",
    path = "departments/reconcile/main.lua",
    queue = "devloop_review_reconcile",
  },
  {
    dept = "fix_reconcile",
    path = "departments/reconcile/main.lua",
    queue = "devloop_fix_reconcile",
  },
  {
    dept = "timeout_reconcile",
    path = "departments/reconcile/main.lua",
    queue = "devloop_timeout_reconcile",
  },
  {
    dept = "rollup_merge",
    path = "departments/rollup_merge/main.lua",
    queue = "devloop_rollup_ready",
  },
}

return {
  test_unsupported_payload_consumers_skip_non_table_payloads = function()
    for _, case in ipairs(cases) do
      for _, payload in ipairs({ false, "foreign-payload", 42 }) do
        local result = t.run_department(case.path, {
          queue = case.queue,
          payload = payload,
        })

        t.eq(result.exit_code, 0)
        t.eq(#result.raises, 0)
      end
    end
  end,

  test_payload_field_returns_nil_for_userdata = function()
    local userdata_payload = assert(io.tmpfile())
    t.eq(core.payload_field(userdata_payload, "dedup_key"), nil)
    userdata_payload:close()
  end,
}
