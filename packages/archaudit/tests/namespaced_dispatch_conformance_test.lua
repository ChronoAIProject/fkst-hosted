local conformance = require("testkit.namespaced_dispatch_conformance")
local helper = require("tests.fire_raiser_helpers")
local t = fkst.test

local function load_department(path, module_name)
  local old_pipeline = pipeline
  local module = require(module_name)
  pipeline = old_pipeline
  return { path = path, module = module }
end

local audit_department = load_department("departments/audit/main.lua", "departments.audit.main")

local function system_idle_payload()
  return {
    schema = "idle-detector.system-idle.v1",
    detected_at = "2026-06-19T01:00:00Z",
    expires_at = "2026-06-19T01:10:00Z",
    source_ref = {
      kind = "host-observe",
      ref = "idle_tick/2026-06-19T01:00:00Z",
    },
  }
end

local function payload_for_queue(_path, queue)
  if queue == "idle-detector.system_idle" then
    return system_idle_payload()
  end
  error("archaudit: no production-shaped queue fixture for " .. tostring(queue))
end

local function opts_for_case()
  return {
    run_opts = {
      env = {
        FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/archaudit/namespaced",
        FKST_DURABLE_ROOT = "/tmp/fkst-packages-test/archaudit/namespaced-durable",
        FKST_GITHUB_REPO = "",
        FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
        ARCHAUDIT_MAX_ISSUES_PER_IDLE = "3",
      },
    },
  }
end

local function observe_facts()
  return {
    schema_version = 1,
    generated_at_ms = 1781830860000,
    source = {},
    limits = { max_deliveries = 500, max_dead_letters = 500 },
    truncated = { deliveries = false, dead_letters = false },
    queues = {},
    deliveries = {},
    dead_letters = {},
  }
end

local function idle_only_departments()
  local spec = {}
  for key, value in pairs(audit_department.module.spec) do
    spec[key] = value
  end
  spec.consumes = { "idle-detector.system_idle" }
  local module = {
    spec = spec,
    pipeline = audit_department.module.make_department({
      github = {
        issue_search = function()
          return { stdout = "[]", stderr = "", exit_code = 0 }
        end,
      },
      observe = {
        facts = observe_facts,
      },
    }).pipeline,
  }
  return conformance.loaded_departments({
    { path = audit_department.path, module = module },
  })
end

return {
  test_idle_dispatch_fixture_accepts_production_namespaced_consumed_queue = function()
    conformance.assert_all_consumed_queues_route({
      t = t,
      package_name = "archaudit",
      package_root = "packages/archaudit",
      departments = idle_only_departments(),
      payload_for_queue = payload_for_queue,
      opts_for_case = opts_for_case,
    })
  end,

  test_audit_poll_namespaced_dispatch_uses_real_fire_raiser = function()
    local root = helper.setup_workspace("namespaced", helper.fire_raiser_child([[
  test_fire_raiser_namespaced_dispatch = function()
    mock_env("owner/repo", "3")
    mock_idle_observe()
    mock_production_github("[]", "[]")
    mock_codex_findings("[]", 0)

    local trace = t.fire_raiser("audit_poll")
    t.eq(trace.source_payload.raiser, "archaudit.audit_poll")
    t.eq(trace.routed_to[1], "archaudit.audit")
    t.eq(trace.consumer_result.status, "accepted")
    t.eq(#trace.raised, 1)
  end,
]]))
    local output = helper.run_child(root)
    t.is_true(output:find("1 passed, 0 failed", 1, true) ~= nil, output)
  end,
}
