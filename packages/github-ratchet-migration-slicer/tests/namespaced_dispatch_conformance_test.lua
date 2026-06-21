local conformance = require("std.namespaced_dispatch_conformance")
local t = fkst.test

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
      test_module_name = "tests.namespaced_dispatch_conformance_test",
      payload_for_queue = payload_for_queue,
    })
  end,
}
