local conformance = require("std.namespaced_dispatch_conformance")
local t = fkst.test

local function source_ref()
  return {
    kind = "external",
    ref = "owner/repo#issue/42",
  }
end

local function entity_payload()
  return {
    schema = "github-proxy.v1",
    type = "issue",
    repo = "owner/repo",
    number = 42,
    title = "Bridge issue",
    url = "https://github.example/owner/repo/issues/42",
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    source_ref = source_ref(),
    dedup_key = "owner/repo#issue#42@2026-06-03T01:02:03Z",
  }
end

local function reply_payload()
  return {
    schema = "autochrono.reply.v1",
    repo = "owner/repo",
    issue_number = 42,
    body = "Draft reply",
    dedup_key = "autochrono:owner/repo#issue/42",
    source_ref = source_ref(),
  }
end

local function payload_for_queue(_path, queue)
  local payloads = {
    ["github-proxy.github_entity_changed"] = entity_payload(),
    ["autochrono.reply"] = reply_payload(),
  }
  local payload = payloads[queue]
  if payload == nil then
    error("github-autochrono: no production-shaped queue fixture for " .. tostring(queue))
  end
  return payload
end

return {
  test_all_departments_accept_production_namespaced_consumed_queues = function()
    conformance.assert_all_consumed_queues_route({
      t = t,
      package_name = "github-autochrono",
      test_module_name = "tests.namespaced_dispatch_conformance_test",
      payload_for_queue = payload_for_queue,
    })
  end,
}
