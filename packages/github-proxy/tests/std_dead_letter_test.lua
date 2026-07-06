-- std module behavior tests are hosted in github-proxy (a flat package, the
-- strictest single-root conformance gate) because the engine test runner only
-- scans <root>/tests and <root>/departments/* (no recursion into std/tests).
local dead_letter = require("workflow.dead_letter")
local error_facts = require("contract.error_facts")
local saga = require("workflow.saga")
local t = fkst.test

local function capture_logs(fn)
  local captured = {}
  local old_log = log
  log = {
    warn = function(message)
      table.insert(captured, tostring(message))
    end,
    info = function(message)
      table.insert(captured, tostring(message))
    end,
    error = function(message)
      table.insert(captured, tostring(message))
    end,
  }

  local ok, result = pcall(fn)
  log = old_log
  if not ok then
    error(result)
  end
  return captured
end

return {
  test_extract_source_ref_and_dedup_key_from_plain_payload = function()
    local payload = {
      source_ref = { kind = "external", ref = "owner/repo#issue/42" },
      dedup_key = "dead-letter/plain",
    }

    t.eq(dead_letter.extract_source_ref(payload), "external:owner/repo#issue/42")
    t.eq(dead_letter.extract_dedup_key(payload), "dead-letter/plain")
  end,

  test_extract_source_ref_and_dedup_key_from_wrapped_payload = function()
    local payload = {
      payload = {
        source_ref = { kind = "external", ref = "owner/repo#pull/7" },
        dedup_key = "dead-letter/wrapped",
      },
    }

    t.eq(dead_letter.extract_source_ref(payload), "external:owner/repo#pull/7")
    t.eq(dead_letter.extract_dedup_key(payload), "dead-letter/wrapped")
  end,

  test_extract_source_ref_and_dedup_key_preserve_nil_behavior = function()
    local payload = {}

    t.eq(dead_letter.extract_source_ref(payload), "")
    t.is_nil(dead_letter.extract_dedup_key(payload))
  end,

  test_shared_department_logs_canonical_l2_failure_fact = function()
    local spec = {
      consumes = { "dead_letter" },
      produces = {},
      stall_window = "2m",
    }
    local module = saga.department(spec, dead_letter.handlers({
      package = "demo-package",
    }))
    local logs = capture_logs(function()
      module.pipeline({
        queue = "dead_letter",
        payload = {
          delivery_id = "delivery/v1/raised/queue/demo.worker/dept/demo.worker/01HY",
          queue = "demo.worker",
          dept = "demo.worker",
          error_class = "worker-failed",
          source_ref = {
            kind = "external",
            ref = "owner/repo#issue/42",
          },
          dedup_key = "demo/dedup",
          attempt = 3,
          error = "worker failed\nwhile handling input",
        },
      })
    end)

    t.eq(module.spec.consumes[1], "dead_letter")
    t.eq(#logs, 1)
    t.eq(
      logs[1],
      "demo-package dept=dead_letter tag=DEAD_LETTER"
        .. " error_class=worker-failed"
        .. " fingerprint=" .. error_facts.error_fingerprint("worker-failed", "demo.worker", "demo.worker", "worker failed\nwhile handling input")
        .. " source_ref=external:owner/repo#issue/42"
        .. " attempt=3"
        .. " terminal=true"
        .. " delivery_id=delivery/v1/raised/queue/demo.worker/dept/demo.worker/01HY"
        .. " queue=demo.worker"
        .. " dead_dept=demo.worker"
        .. " source_ref=external:owner/repo#issue/42"
        .. " dedup_key=demo/dedup"
        .. " attempt=3"
        .. " error=worker failed while handling input"
    )
  end,
}
