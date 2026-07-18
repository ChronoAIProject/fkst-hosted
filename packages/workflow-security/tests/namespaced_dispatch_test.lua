-- Namespaced-dispatch conformance (unit level): each department must accept its
-- consumed queues under their PRODUCTION namespaced names (package-prefixed) and
-- reject foreign queues -- the idle_gate bug shape. We exercise the accept handlers
-- the kernel + this adapter compose, so no bare own-queue compare can slip in.
local reconcile = require("workflow.engine.reconcile")
local intake = require("intake")
local records = require("records")
local security_logic = require("security_logic")
local t = fkst.test

local function stub_discovery()
  return {
    list_scopes = function()
      return {}
    end,
    origin_of = function(scope)
      return scope.origin
    end,
    read_current = function()
      return { state = "OPEN" }
    end,
    latest_terminal = function()
      return nil
    end,
    latest_blueprint = function()
      return nil
    end,
  }
end

local function intake_handlers()
  return intake.build({
    marker = { build_blueprint_marker = function()
      return "<!-- marker -->"
    end },
    discovery = stub_discovery(),
    blueprint = records.BLUEPRINT,
    workflow_id = security_logic.WORKFLOW_ID,
    repo = "owner/repo",
    materialization_tick_queue = security_logic.MATERIALIZATION_TICK_QUEUE,
    consumes = { security_logic.TICK_QUEUE },
  })
end

local function reconcile_handlers()
  return reconcile.handlers({
    tick_queue = security_logic.MATERIALIZATION_TICK_QUEUE,
    platform = {},
    executor = {},
    completion = {},
    catalog = {},
  })
end

return {
  test_intake_accepts_namespaced_own_queues = function()
    local handlers = intake_handlers()
    t.is_true(handlers.accept({ queue = "workflow-security.workflow_security_tick" }))
    -- bare (non-namespaced) form must also route for local dispatch
    t.is_true(handlers.accept({ queue = "workflow_security_tick" }))
  end,

  test_intake_rejects_foreign_queue = function()
    local handlers = intake_handlers()
    t.is_true(not handlers.accept({ queue = "github-devloop-intake.devloop_intake_candidate" }))
    t.is_true(not handlers.accept({ queue = "some-other.queue" }))
  end,

  test_reconcile_accepts_namespaced_materialization_tick = function()
    local handlers = reconcile_handlers()
    t.is_true(handlers.accept({ queue = "workflow-security.workflow_security_materialization_tick" }))
    t.is_true(handlers.accept({ queue = "workflow_security_materialization_tick" }))
  end,

  test_reconcile_rejects_foreign_queue = function()
    local handlers = reconcile_handlers()
    t.is_true(not handlers.accept({ queue = "workflow-security.workflow_security_tick" }))
    t.is_true(not handlers.accept({ queue = "github-proxy.github_entity_changed" }))
  end,
}
