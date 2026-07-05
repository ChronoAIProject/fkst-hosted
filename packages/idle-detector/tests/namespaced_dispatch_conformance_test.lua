local conformance = require("testkit.namespaced_dispatch_conformance")
local github_fake = require("forge.github_fake")
local t = fkst.test

local function load_department(path, module_name)
  local old_pipeline = pipeline
  local module = require(module_name)
  pipeline = old_pipeline
  local github = github_fake.new(github_fake.model({}))
  return {
    path = path,
    module = module.make_department({
      github = github,
      read_env = function(name)
        if name == "FKST_GITHUB_REPO" then
          return "owner/repo"
        end
        if name == "FKST_GITHUB_BOT_LOGIN" then
          return "fkst-test-bot"
        end
        return nil
      end,
      now = function()
        return 1781830860
      end,
    }),
  }
end

local departments = conformance.loaded_departments({
  load_department("departments/idle_gate/main.lua", "departments.idle_gate.main"),
})

local function idle_tick_payload()
  local slot = "1970-01-01T00:00:00Z"
  return {
    schema = "idle-detector.idle-tick.v1",
    slot = slot,
    source_ref = {
      kind = "cron",
      ref = "idle-detector/idle_poll/" .. slot,
    },
  }
end

local function payload_for_queue(_path, queue)
  if queue == "idle_tick" then
    return idle_tick_payload()
  end
  error("idle-detector: no production-shaped queue fixture for " .. tostring(queue))
end

local function opts_for_case(_path, _queue, event)
  event.ts = event.payload.slot
  return {
    run_opts = {
      env = {
        FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/idle-detector/namespaced",
      },
    },
  }
end

return {
  test_all_departments_accept_production_namespaced_consumed_queues = function()
    conformance.assert_all_consumed_queues_route({
      t = t,
      package_name = "idle-detector",
      package_root = "packages/idle-detector",
      departments = departments,
      payload_for_queue = payload_for_queue,
      opts_for_case = opts_for_case,
    })
  end,
}
