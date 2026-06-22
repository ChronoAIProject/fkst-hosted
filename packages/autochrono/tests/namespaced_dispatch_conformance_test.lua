local conformance = require("testkit.namespaced_dispatch_conformance")
local t = fkst.test

local function load_department(path, module_name)
  local old_pipeline = pipeline
  local module = require(module_name)
  pipeline = old_pipeline
  return { path = path, module = module }
end

local departments = conformance.loaded_departments({
  load_department("departments/propose/main.lua", "departments.propose.main"),
  load_department("departments/reply/main.lua", "departments.reply.main"),
})

local function issue_payload()
  return {
    schema = "autochrono.issue.v1",
    repo = "owner/repo",
    issue_number = 42,
    title = "Bridge issue",
    url = "https://github.example/owner/repo/issues/42",
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    source_ref = {
      kind = "external",
      ref = "owner/repo#issue/42",
    },
    dedup_key = "owner/repo#issue#42@2026-06-03T01:02:03Z",
  }
end

local function reached_payload()
  return {
    schema = "consensus.consensus_reached.v1",
    proposal_id = "autochrono/issue/owner/repo/42",
    decision = "approve",
    body = "Thanks for opening this. I will review the details and follow up with the next concrete step.",
    dedup_key = "consensus:autochrono/issue/owner/repo/42/2026-06-03T01-02-03Z",
    source_ref = {
      kind = "external",
      ref = "owner/repo#issue/42",
    },
  }
end

local function payload_for_queue(_path, queue)
  local payloads = {
    issue = issue_payload(),
    ["consensus.consensus_reached"] = reached_payload(),
  }
  local payload = payloads[queue]
  if payload == nil then
    error("autochrono: no production-shaped queue fixture for " .. tostring(queue))
  end
  return payload
end

local function opts_for_case(_path, queue)
  return {
    run_opts = {
      env = {
        FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/autochrono/namespaced-" .. tostring(queue):gsub("[^%w._-]", "_"),
      },
    },
  }
end

return {
  test_all_departments_accept_production_namespaced_consumed_queues = function()
    conformance.assert_all_consumed_queues_route({
      t = t,
      package_name = "autochrono",
      package_root = "packages/autochrono",
      departments = departments,
      payload_for_queue = payload_for_queue,
      opts_for_case = opts_for_case,
    })
  end,
}
