local entity_lib = require("devloop.entity")
local t = fkst.test
local core = require("core")

local package_root = "packages/github-devloop-intake"

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
    error("github-devloop-intake: department discovery failed")
  end
  return result
end

local function load_department_spec(path)
  local old_pipeline = pipeline
  local module = require(tostring(path):gsub("/", "."):gsub("%.lua$", ""))
  pipeline = old_pipeline
  if type(module) ~= "table" or type(module.spec) ~= "table" then
    error("github-devloop-intake: department spec missing for " .. tostring(path))
  end
  return module.spec
end

local function production_queue_name(queue)
  if tostring(queue):find("%.", 1, false) ~= nil then
    return queue
  end
  return "github-devloop-intake." .. tostring(queue)
end

local function payload_for_queue(queue)
  local payloads = {
    ["github-proxy.github_entity_changed"] = {
      schema = "github-proxy.v1",
      type = "issue",
      repo = "owner/repo",
      number = 42,
      title = "Namespaced dispatch probe",
      state = "CLOSED",
      labels = {},
      updated_at = "2026-06-03T01:02:03Z",
      dedup_key = "owner/repo#issue#42@2026-06-03T01:02:03Z",
      source_ref = entity_lib.issue_source_ref("owner/repo", 42),
    },
  }
  local payload = payloads[queue]
  if payload == nil then
    error("github-devloop-intake: no production-shaped queue fixture for " .. tostring(queue))
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

local function assert_no_unsupported_queue_fallthrough(path, queue, _ok, err, logs)
  local text = tostring(err or "") .. "\n" .. tostring(logs or "")
  if text:find("consumed-queue-unrouted", 1, true) ~= nil then
    error("github-devloop-intake: consumed queue is unrouted for " .. path .. " queue=" .. queue .. ": " .. text)
  end
  if text:find("unsupported event payload", 1, true) ~= nil
    or text:find("skip-foreign(payload)", 1, true) ~= nil
    or text:find("skip-foreign(source_ref)", 1, true) ~= nil then
    error("github-devloop-intake: production-shaped consumed queue fell through unsupported path for " .. path .. " queue=" .. queue .. ": " .. text)
  end
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
        local ok, err, logs = run_department_with_logs(path, event)
        assert_no_unsupported_queue_fallthrough(path, queue, ok, err, logs)
      end
    end
  end,
}
