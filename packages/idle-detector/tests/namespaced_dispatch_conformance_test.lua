local conformance = require("testkit.namespaced_dispatch_conformance")
local t = fkst.test
local observe_bin = "/tmp/fkst-framework"
local observe_durable_root = "/tmp/fkst-durable"

local function load_department(path, module_name)
  local old_pipeline = pipeline
  local module = require(module_name)
  pipeline = old_pipeline
  return { path = path, module = module }
end

local departments = conformance.loaded_departments({
  load_department("departments/idle_gate/main.lua", "departments.idle_gate.main"),
})

local function observe_json()
  return table.concat({
    '{"schema_version":1',
    ',"generated_at_ms":1781830860000',
    ',"source":{"durable_root":"/tmp/fkst-durable","database":"/tmp/fkst-durable/delivery.redb","read_semantics":"single read transaction","history_semantics":"delivery queue snapshot only"}',
    ',"limits":{"max_deliveries":500,"max_dead_letters":500}',
    ',"truncated":{"deliveries":false,"dead_letters":false}',
    ',"queues":[{"queue":"proposal","depth":0,"pending":0,"in_flight":0,"retrying":0,"oldest_pending_age_ms":null}]',
    ',"deliveries":[]',
    ',"dead_letters":[]',
    "}",
  }, "")
end

local function idle_tick_payload()
  local slot = "2026-06-19T01:00:00Z"
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

local function mock_observe()
  t.mock_command('printf %s "$BIN"', { stdout = observe_bin, stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DURABLE_ROOT"', { stdout = observe_durable_root, stderr = "", exit_code = 0 })
  t.mock_command(observe_bin .. " observe --durable-root " .. observe_durable_root .. " --json", {
    stdout = observe_json(),
    stderr = "",
    exit_code = 0,
  })
end

local function opts_for_case(_path, _queue, event)
  event.ts = event.payload.slot
  mock_observe()
  return {
    run_opts = {
      env = {
        FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/idle-detector/namespaced",
        FKST_DURABLE_ROOT = "/tmp/fkst-packages-test/idle-detector/namespaced-durable",
      },
    },
    before_replay = function()
      mock_observe()
    end,
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
