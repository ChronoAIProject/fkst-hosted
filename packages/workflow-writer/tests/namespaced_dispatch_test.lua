-- Namespaced-dispatch conformance (unit level): each department must accept its
-- consumed queues under their PRODUCTION namespaced names (package-prefixed) and reject
-- foreign queues -- the idle_gate bug shape. We exercise the accept handlers the kernel
-- + this adapter compose, so no bare own-queue compare can slip in. Crucially, the
-- select department must REJECT the dev intake candidate seam (INTAKE_POLICY_SET).
local reconcile = require("workflow.engine.reconcile")
local intake = require("intake")
local records = require("records")
local authoring = require("authoring")
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
    workflow_id = authoring.WORKFLOW_ID,
    repo = "owner/repo",
    materialization_tick_queue = authoring.MATERIALIZATION_TICK_QUEUE,
    consumes = { authoring.TICK_QUEUE },
  })
end

local function reconcile_handlers()
  return reconcile.handlers({
    tick_queue = authoring.MATERIALIZATION_TICK_QUEUE,
    platform = {},
    executor = {},
    completion = {},
    catalog = {},
  })
end

return {
  test_intake_accepts_namespaced_own_queues = function()
    local handlers = intake_handlers()
    t.is_true(handlers.accept({ queue = "workflow-writer.workflow_writer_tick" }))
    t.is_true(handlers.accept({ queue = "workflow_writer_tick" }))
  end,

  test_intake_rejects_dev_candidate_and_foreign = function()
    local handlers = intake_handlers()
    t.is_true(not handlers.accept({ queue = "github-devloop-intake.devloop_intake_candidate" }))
    t.is_true(not handlers.accept({ queue = "some-other.queue" }))
  end,

  test_reconcile_accepts_namespaced_materialization_tick = function()
    local handlers = reconcile_handlers()
    t.is_true(handlers.accept({ queue = "workflow-writer.workflow_writer_materialization_tick" }))
    t.is_true(handlers.accept({ queue = "workflow_writer_materialization_tick" }))
  end,

  test_reconcile_rejects_foreign_queue = function()
    local handlers = reconcile_handlers()
    t.is_true(not handlers.accept({ queue = "workflow-writer.workflow_writer_tick" }))
    t.is_true(not handlers.accept({ queue = "github-proxy.github_entity_changed" }))
  end,
}
