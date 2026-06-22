local conformance = require("testkit.namespaced_dispatch_conformance")
local t = fkst.test

local function load_department(path, module_name)
  local old_pipeline = pipeline
  local module = require(module_name)
  pipeline = old_pipeline
  return { path = path, module = module }
end

local departments = conformance.loaded_departments({
  load_department("departments/external_pr_intake/main.lua", "departments.external_pr_intake.main"),
})

local function payload_for_queue(_path, queue)
  local payloads = {
    external_pr_scan = {
      schema = "github-external-pr-intake.scan.v1",
    },
    external_pr_candidate = {
      schema = "github-external-pr-intake.v1",
      repo = "owner/repo",
      number = 7,
      updated_at = "2026-06-03T01:02:03Z",
      dedup_key = "github-external-pr-intake/owner/repo/7",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    },
  }
  local payload = payloads[queue]
  if payload == nil then
    error("github-external-pr-intake: no production-shaped queue fixture for " .. tostring(queue))
  end
  return payload
end

return {
  test_all_departments_accept_production_namespaced_consumed_queues = function()
    conformance.assert_all_consumed_queues_route({
      t = t,
      package_name = "github-external-pr-intake",
      package_root = "packages/github-external-pr-intake",
      departments = departments,
      payload_for_queue = payload_for_queue,
    })
  end,
}
