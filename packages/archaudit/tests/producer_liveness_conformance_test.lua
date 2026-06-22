local conformance = require("contract.producer_liveness_conformance")
local testing = require("contract.testing")
local github_fake = require("std.github_fake")
local core = require("core")
local audit_main = require("departments.audit.main")
local t = fkst.test

local function mock_env()
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$ARCHAUDIT_MAX_ISSUES_PER_IDLE"', { stdout = "3", stderr = "", exit_code = 0 })
end

local function mock_busy_observe()
  t.mock_command('fkst-framework observe --durable-root "$FKST_DURABLE_ROOT" --json', {
    stdout = table.concat({
      '{"schema_version":1',
      ',"generated_at_ms":1781830860000',
      ',"source":{"durable_root":"/tmp/fkst-durable","database":"/tmp/fkst-durable/delivery.redb","read_semantics":"single read transaction","history_semantics":"delivery queue snapshot only"}',
      ',"limits":{"max_deliveries":500,"max_dead_letters":500}',
      ',"truncated":{"deliveries":false,"dead_letters":false}',
      ',"queues":[{"queue":"proposal","depth":1,"pending":1,"in_flight":0,"retrying":0,"oldest_pending_age_ms":1000}]',
      ',"deliveries":[]',
      ',"dead_letters":[]',
      "}",
    }, ""),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_codex_findings()
  t.mock_command("codex exec", {
    stdout = '[{"file":"packages/archaudit/core.lua","line":1,"rule":"SRP","why":"Core has one concrete issue.","suggested_fix":"Move the local helper."}]',
    stderr = "",
    exit_code = 0,
  })
end

local function fake_department()
  local github = github_fake.new(github_fake.model())
  function github.label_list(_repo, _timeout)
    return { stdout = "[]", stderr = "", exit_code = 0 }
  end
  return audit_main.make_department({ github = github, git = nil })
end

local function due_event()
  return {
    queue = "archaudit.audit_due",
    ts = "2026-06-19T01:00:00Z",
    payload = {
      schema = "archaudit.audit-due.v1",
      source_ref = {
        kind = "cron",
        ref = "archaudit/audit_due/2026-06-19T01:00:00Z",
      },
    },
  }
end

local function run_at(seconds, fn)
  local previous_now = now
  now = function()
    return seconds
  end
  local ok, result = pcall(fn)
  now = previous_now
  if not ok then
    error(result, 0)
  end
  return result
end

return {
  test_declared_audit_producer_makes_progress_under_busy_adversary = function()
    mock_env()
    mock_busy_observe()
    mock_codex_findings()
    run_at(core.iso_timestamp_epoch_seconds("2026-06-19T01:01:00Z"), function()
      conformance.assert_declared_producer_progress({
        t = t,
        package_core = core,
        producer_id = "archaudit.audit",
        department_for_delivery = function(_contract, _attempt)
          return fake_department()
        end,
        event_for_contract = function(_contract, _attempt)
          return due_event()
        end,
        before_delivery = function(_contract, _attempt)
          mock_env()
          mock_busy_observe()
          mock_codex_findings()
        end,
      })
    end)
  end,

  test_declared_audit_producer_contract_fails_silent_busy_skip = function()
    local silent_department = {
      spec = {
        consumes = { "audit_due" },
        produces = { "github-proxy.github_issue_create_request" },
      },
      pipeline = function(_event)
        return nil
      end,
    }
    local ok, err = pcall(function()
      conformance.assert_declared_producer_progress({
        t = t,
        package_core = core,
        producer_id = "archaudit.audit",
        department = silent_department,
        event_for_contract = function(_contract, _attempt)
          return due_event()
        end,
      })
    end)
    t.eq(ok, false)
    t.is_true(tostring(err):find("produced no output or escalation", 1, true) ~= nil)
  end,
}
