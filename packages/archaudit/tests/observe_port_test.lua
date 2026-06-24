local testing = require("testkit.testing")
local github_fake = require("forge.github_fake")
local audit_main = require("departments.audit.main")
local observe_port = require("departments.audit.observe_port")
local t = fkst.test

local function observe_idle()
  return {
    schema_version = 1,
    generated_at_ms = 1781830860000,
    source = {
      durable_root = "/tmp/fkst-durable",
      database = "/tmp/fkst-durable/delivery.redb",
      read_semantics = "single read transaction",
      history_semantics = "delivery queue snapshot only",
    },
    limits = { max_deliveries = 500, max_dead_letters = 500 },
    truncated = { deliveries = false, dead_letters = false },
    queues = {
      { queue = "proposal", depth = 0, pending = 0, in_flight = 0, retrying = 0 },
    },
    deliveries = json.decode("[]"),
    dead_letters = json.decode("[]"),
  }
end

local function fresh_idle_event()
  return {
    queue = "idle-detector.system_idle",
    ts = "2026-06-19T01:00:00Z",
    payload = {
      schema = "idle-detector.system-idle.v1",
      detected_at = "2026-06-19T01:00:00Z",
      expires_at = "2026-06-19T01:10:00Z",
      source_ref = { kind = "host-observe", ref = "idle_tick/2026-06-19T01:00:00Z" },
    },
  }
end

local function mock_env()
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$ARCHAUDIT_MAX_ISSUES_PER_IDLE"', { stdout = "3", stderr = "", exit_code = 0 })
end

return {
  test_audit_observe_adapter_rejects_malformed_injected_snapshot = function()
    t.mock_observe("not facts")
    t.raises(function() observe_port.facts() end)
  end,

  test_audit_observe_adapter_reports_malformed_snapshot_error_class = function()
    t.mock_observe("not facts")
    local ok, err = pcall(function()
      observe_port.facts()
    end)
    t.eq(ok, false)
    t.is_true(tostring(err):find("archaudit: observe-malformed", 1, true) ~= nil)
  end,

  test_audit_observe_adapter_consumes_injected_snapshot = function()
    t.mock_observe(observe_idle())
    local observed = observe_port.facts()
    t.eq(observed.schema_version, 1)
    t.eq(observed.generated_at_ms, 1781830860000)
  end,

  test_audit_observe_adapter_accepts_generic_options = function()
    t.mock_observe(observe_idle())
    local observed = observe_port.facts({ limit = 10 })
    t.eq(observed.schema_version, 1)
    t.eq(#observed.queues, 1)
  end,

  test_audit_department_consumes_injected_observe_port = function()
    mock_env()
    t.mock_command("codex exec", { stdout = "[]", stderr = "", exit_code = 0 })

    local model = github_fake.model()
    local github = github_fake.new(model)
    function github.issue_search(_repo, _query, _fields, _timeout)
      return { stdout = "[]", stderr = "", exit_code = 0 }
    end
    function github.label_list(_repo, _timeout)
      return { stdout = "[]", stderr = "", exit_code = 0 }
    end

    local observe_calls = 0
    local dept = audit_main.make_department({
      github = github,
      git = nil,
      observe = {
        facts = function()
          observe_calls = observe_calls + 1
          return observe_idle()
        end,
      },
    })

    local result = testing.run_fake(dept, fresh_idle_event())
    t.eq(observe_calls, 1)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_create_request")
  end,
}
