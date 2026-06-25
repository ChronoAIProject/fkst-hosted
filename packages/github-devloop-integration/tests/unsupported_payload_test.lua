local t = fkst.test
local core = require("core")

local package_root = "packages/github-devloop-integration"

local function department_paths()
  local root = package_root
  local result = {}
  local find = assert(io.popen("find " .. root .. "/departments -mindepth 2 -maxdepth 2 -name main.lua | sort"))
  for path in find:lines() do
    local rel = path:sub(#root + 2)
    table.insert(result, rel)
  end
  local ok = find:close()
  if ok == false then
    error("github-devloop: department discovery failed")
  end
  return result
end

local function load_department_spec(path)
  local old_pipeline = pipeline
  local module = require(tostring(path):gsub("/", "."):gsub("%.lua$", ""))
  pipeline = old_pipeline
  if type(module) ~= "table" or type(module.spec) ~= "table" then
    error("github-devloop: department spec missing for " .. tostring(path))
  end
  return module.spec
end

local function production_queue_name(queue)
  if tostring(queue):find("%.", 1, false) ~= nil then
    return queue
  end
  return "github-devloop-integration." .. tostring(queue)
end

local function payload_for_queue(queue)
  local payloads = {
    cache_seed = {
      key = "github-devloop-integration/test-cache-seed",
      value = "1",
    },
    devloop_branch_tick = { schema = "github-devloop.branch-tick.v1" },
    devloop_rollup_ready = core.rollup_ready_payload("owner/repo", "dev", "integration/dev", 7, "def456"),
    devloop_sync_conflict = {
      schema = "github-devloop.v1",
      repo = "owner/repo",
      upstream_branch = "dev",
      integration_branch = "integration/dev",
      upstream_sha = "abc123",
      integration_sha = "def456",
      dedup_key = core.branch_sync_dedup_key("owner/repo", "dev", "integration/dev", "abc123"),
      source_ref = core.branch_sync_source_ref("owner/repo", "dev", "integration/dev"),
    },
  }
  local payload = payloads[queue]
  if payload == nil then
    error("github-devloop: no production-shaped queue fixture for " .. tostring(queue))
  end
  return payload
end

local function run_department_with_logs(path, event)
  local result = t.run_department(path, event)
  t.is_true(type(result) == "table")
  return result.exit_code == 0, tostring(result.error or ""), table.concat({
    tostring(result.error or ""),
  }, "\n")
end

local function assert_no_unsupported_queue_fallthrough(path, queue, err, logs)
  local text = tostring(err or "") .. "\n" .. tostring(logs or "")
  if text:find("consumed-queue-unrouted", 1, true) ~= nil then
    error("github-devloop: consumed queue is unrouted for " .. path .. " queue=" .. queue .. ": " .. text)
  end
  if text:find("unsupported event payload", 1, true) ~= nil
    or text:find("unsupported sync conflict payload", 1, true) ~= nil
    or text:find("skip-foreign(payload)", 1, true) ~= nil
    or text:find("skip-foreign(pr)", 1, true) ~= nil
    or text:find("skip-foreign(proposal_id)", 1, true) ~= nil
    or text:find("skip-foreign(source_ref)", 1, true) ~= nil then
    error("github-devloop: production-shaped consumed queue fell through unsupported path for " .. path .. " queue=" .. queue .. ": " .. text)
  end
end

local cases = {
  {
    dept = "sync_conflict",
    path = "departments/sync_conflict/main.lua",
    queue = "devloop_sync_conflict",
  },
  {
    dept = "rollup_merge",
    path = "departments/rollup_merge/main.lua",
    queue = "devloop_rollup_ready",
  },
}

local function expects_non_table_payload_fail_closed(dept)
  return dept == "rollup_merge"
end

return {
  test_all_departments_accept_production_namespaced_consumed_queues = function()
    for _, path in ipairs(department_paths()) do
      local spec = load_department_spec(path)
      for _, queue in ipairs(spec.consumes or {}) do
        local event = {
          queue = production_queue_name(queue),
          payload = payload_for_queue(queue),
        }
        local _, err, logs = run_department_with_logs(path, event)
        assert_no_unsupported_queue_fallthrough(path, queue, err, logs)
      end
    end
  end,

  test_unsupported_payload_consumers_handle_non_table_payloads_by_queue_contract = function()
    for _, case in ipairs(cases) do
      for _, payload in ipairs({ false, "foreign-payload", 42 }) do
        local result = t.run_department(case.path, {
          queue = case.queue,
          payload = payload,
        })

        if expects_non_table_payload_fail_closed(case.dept) then
          t.is_true(result.exit_code ~= 0)
          t.is_true(tostring(result.error or ""):find(
            "github-devloop: rollup_merge unsupported devloop_rollup_ready payload",
            1,
            true
          ) ~= nil)
        else
          t.eq(result.exit_code, 0)
        end
        t.eq(#result.raises, 0)
      end
    end
  end,
}
