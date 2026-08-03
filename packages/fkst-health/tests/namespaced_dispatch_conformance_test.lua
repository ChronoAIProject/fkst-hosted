-- Every queue this package's departments declare in `consumes` must route under its
-- PRODUCTION namespaced name ("fkst-health.health_tick"), not just the bare local
-- one. A department that only recognises the bare name accepts nothing in production
-- and the session reports nothing at all.
local conformance = require("testkit.namespaced_dispatch_conformance")
local t = fkst.test

local function load_department(path, module_name)
  -- require() side-effects _G.pipeline; restore it so loading a department for a test
  -- cannot clobber the production pipeline the harness installed.
  local old_pipeline = pipeline
  local module = require(module_name)
  pipeline = old_pipeline
  return {
    path = path,
    module = module.make_department({}),
  }
end

local departments = conformance.loaded_departments({
  load_department("departments/health_report/main.lua", "departments.health_report.main"),
})

local function health_tick_payload()
  local slot = "1970-01-01T00:00:00Z"
  return {
    schema = "fkst-health.health-tick.v1",
    slot = slot,
    source_ref = {
      kind = "cron",
      ref = "fkst-health/health_poll/" .. slot,
    },
  }
end

local function payload_for_queue(_path, queue)
  if queue == "health_tick" then
    return health_tick_payload()
  end
  error("fkst-health: no production-shaped queue fixture for " .. tostring(queue))
end

local function opts_for_case(_path, _queue, event)
  event.ts = event.payload.slot
  -- FKST_SESSION_ID is deliberately absent. The department then takes its "no session
  -- identity, no report" path, which routes and logs without spawning a codex or
  -- writing anything. What this test owns is that the production namespaced queue
  -- ROUTES; the write path is covered end to end against fakes in
  -- integration_health_report_test.lua.
  return {
    run_opts = {
      env = {
        FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/fkst-health/namespaced",
      },
    },
  }
end

return {
  test_all_departments_accept_production_namespaced_consumed_queues = function()
    -- Belt and braces: this department runs here with PRODUCTION ports, so pin the
    -- codex command even though the path above never reaches it. A test must never be
    -- one environment change away from spawning a real model run.
    t.mock_command("codex exec", { stdout = "", stderr = "", exit_code = 1 })

    conformance.assert_all_consumed_queues_route({
      t = t,
      package_name = "fkst-health",
      package_root = "packages/fkst-health",
      departments = departments,
      payload_for_queue = payload_for_queue,
      opts_for_case = opts_for_case,
    })
  end,
}
