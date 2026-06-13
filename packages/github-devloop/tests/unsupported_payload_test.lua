local t = fkst.test
local core = require("core")

local function package_root()
  local source = package.searchpath("tests.unsupported_payload_test", package.path)
  return source:match("(.+)/tests/unsupported_payload_test%.lua$")
end

local function read_file(path)
  local handle = assert(io.open(path, "r"))
  local body = handle:read("*a")
  handle:close()
  return body
end

local function department_source(path)
  return read_file(package_root() .. "/" .. path)
end

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
  test_consumed_queue_dispatch_accepts_namespaced_declared_queues = function()
    local routed = {}
    local spec = {
      consumes = { "devloop_ready", "devloop_ready_session" },
    }
    local handled = core.dispatch_consumed_queue("test", spec, {
      queue = "github-devloop.devloop_ready_session",
      payload = {},
    }, {
      devloop_ready = function()
        table.insert(routed, "ready")
      end,
      devloop_ready_session = function()
        table.insert(routed, "ready-session")
      end,
    })

    t.eq(handled, true)
    t.eq(routed[1], "ready-session")
  end,

  test_consumed_queue_dispatch_fail_closed_when_declared_queue_is_unrouted = function()
    t.raises(function()
      core.dispatch_consumed_queue("test", {
        consumes = { "devloop_ready", "devloop_ready_session" },
      }, {
        queue = "github-devloop.devloop_ready_session",
        payload = {},
      }, {
        devloop_ready = function() end,
      })
    end)
  end,

  test_consumed_queue_dispatch_skips_foreign_queue_without_error = function()
    local handled = core.dispatch_consumed_queue("test", {
      consumes = { "devloop_ready" },
    }, {
      queue = "github-proxy.github_entity_changed",
      payload = {},
    }, {
      devloop_ready = function()
        error("github-devloop: unexpected foreign dispatch")
      end,
    })

    t.eq(handled, false)
  end,

  test_event_queue_matches_namespaced_session_queue = function()
    t.eq(core.event_queue_matches({ queue = "github-devloop.devloop_ready_session" }, "devloop_ready_session"), true)
    t.eq(core.event_queue_matches({ queue = "devloop_ready_session" }, "devloop_ready_session"), true)
    t.eq(core.event_queue_matches({ queue = "github-devloop.devloop_ready" }, "devloop_ready_session"), false)
  end,

  test_queue_dispatching_departments_enumerate_every_consumed_queue = function()
    for _, case in ipairs({
      {
        dept = "implement",
        path = "departments/implement/main.lua",
        consumes = { "devloop_ready", "devloop_ready_session" },
      },
      {
        dept = "merge",
        path = "departments/merge/main.lua",
        consumes = { "devloop_merge_ready", "devloop_merge_queue_tick" },
      },
    }) do
      local source = department_source(case.path)
      t.is_true(source:find("core.dispatch_consumed_queue", 1, true) ~= nil)
      t.is_nil(source:find("event.queue ==", 1, true))
      for _, queue in ipairs(case.consumes) do
        t.is_true(source:find(queue .. "%s*=", 1, false) ~= nil)
      end
    end
  end,

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
