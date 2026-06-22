local conformance = require("testkit.namespaced_dispatch_conformance")
local t = fkst.test

local function load_department(path, module_name)
  local old_pipeline = pipeline
  local module = require(module_name)
  pipeline = old_pipeline
  return { path = path, module = module }
end

local departments = conformance.loaded_departments({
  load_department("departments/ratchet_migration_driver/main.lua", "departments.ratchet_migration_driver.main"),
})

local function payload_for_queue(_path, queue)
  if queue == "ratchet_migration_poll" then
    return {
      schema = "github-ratchet-migration-slicer.ratchet-migration-poll.v1",
      ratchet = "saga-handler",
    }
  end
  error("github-ratchet-migration-slicer: no production-shaped queue fixture for " .. tostring(queue))
end

return {
  test_all_departments_accept_production_namespaced_consumed_queues = function()
    conformance.assert_all_consumed_queues_route({
      t = t,
      package_name = "github-ratchet-migration-slicer",
      package_root = "packages/github-ratchet-migration-slicer",
      departments = departments,
      payload_for_queue = payload_for_queue,
    })
  end,
}
