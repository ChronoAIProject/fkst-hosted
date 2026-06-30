local payloads_builders = require("devloop.payloads.builders")
local t = fkst.test
local core = require("core")

local package_root = "packages/github-devloop-intake-default"

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
    error("github-devloop-intake-default: department discovery failed")
  end
  return result
end

local function load_department_spec(path)
  local old_pipeline = pipeline
  local module = require(tostring(path):gsub("/", "."):gsub("%.lua$", ""))
  pipeline = old_pipeline
  if type(module) ~= "table" or type(module.spec) ~= "table" then
    error("github-devloop-intake-default: department spec missing for " .. tostring(path))
  end
  return module.spec
end

local function production_queue_name(queue)
  if tostring(queue):find("%.", 1, false) ~= nil then
    return queue
  end
  return "github-devloop-intake-default." .. tostring(queue)
end

local function payload_for_queue(queue)
  local payloads = {
    ["github-devloop-intake.devloop_intake_candidate"] = payloads_builders.build_devloop_intake_candidate_payload(core, "owner/repo", "42", "2026-06-03T01:02:03Z"),
  }
  local payload = payloads[queue]
  if payload == nil then
    error("github-devloop-intake-default: no production-shaped queue fixture for " .. tostring(queue))
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
    error("github-devloop-intake-default: consumed queue is unrouted for " .. path .. " queue=" .. queue .. ": " .. text)
  end
  if text:find("unsupported event payload", 1, true) ~= nil
    or text:find("skip-foreign(payload)", 1, true) ~= nil
    or text:find("skip-foreign(source_ref)", 1, true) ~= nil then
    error("github-devloop-intake-default: production-shaped consumed queue fell through unsupported path for " .. path .. " queue=" .. queue .. ": " .. text)
  end
end

local cases = {
  {
    dept = "intake_judge",
    path = "departments/intake_judge/main.lua",
    queue = "github-devloop-intake.devloop_intake_candidate",
  },
}

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

  test_unsupported_payload_consumers_skip_non_table_payloads = function()
    for _, case in ipairs(cases) do
      for _, payload in ipairs({ false, "foreign-payload", 42 }) do
        local result = t.run_department(case.path, {
          queue = case.queue,
          payload = payload,
        })

        t.eq(result.exit_code, 0)
        t.eq(#result.raises, 0)
      end
    end
  end,
}
