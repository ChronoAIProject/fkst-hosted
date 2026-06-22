local conformance = require("contract.namespaced_dispatch_conformance")
local t = fkst.test

local function load_department(path, module_name)
  local old_pipeline = pipeline
  local module = require(module_name)
  pipeline = old_pipeline
  return { path = path, module = module }
end

local departments = conformance.loaded_departments({
  load_department("departments/audit/main.lua", "departments.audit.main"),
})

local function system_idle_payload()
  return {
    schema = "idle-detector.system-idle.v1",
    detected_at = "2026-06-19T01:00:00Z",
    expires_at = "2026-06-19T01:10:00Z",
    source_ref = {
      kind = "host-observe",
      ref = "idle_tick/2026-06-19T01:00:00Z",
    },
  }
end

local function payload_for_queue(_path, queue)
  if queue == "idle-detector.system_idle" then
    return system_idle_payload()
  end
  error("archaudit: no production-shaped queue fixture for " .. tostring(queue))
end

local function opts_for_case()
  return {
    run_opts = {
      env = {
        FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/archaudit/namespaced",
        FKST_DURABLE_ROOT = "/tmp/fkst-packages-test/archaudit/namespaced-durable",
        FKST_GITHUB_REPO = "",
        ARCHAUDIT_MAX_ISSUES_PER_IDLE = "3",
      },
    },
    before_replay = function()
      local core = require("core")
      local old_observe = core.observe
      core.observe = function()
        return {
          schema_version = 1,
          generated_at_ms = 1781830860000,
          source = {},
          limits = { max_deliveries = 500, max_dead_letters = 500 },
          truncated = { deliveries = false, dead_letters = false },
          queues = {},
          deliveries = {},
          dead_letters = {},
        }
      end
      return function()
        core.observe = old_observe
      end
    end,
  }
end

return {
  test_all_departments_accept_production_namespaced_consumed_queues = function()
    conformance.assert_all_consumed_queues_route({
      t = t,
      package_name = "archaudit",
      package_root = "packages/archaudit",
      departments = departments,
      payload_for_queue = payload_for_queue,
      opts_for_case = opts_for_case,
    })
  end,
}
