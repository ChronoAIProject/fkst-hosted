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

local function event(payload)
  return {
    queue = "dead_letter",
    payload = payload,
  }
end

local function dead_payload()
  return {
    delivery_id = "delivery/v3/raised/queue/consensus.consensus_reached/dept/github-devloop-pr.review_result/01HY",
    queue = "consensus.consensus_reached",
    dept = "github-devloop-pr.review_result",
    error_class = "review-result-failed",
    source_ref = {
      kind = "external",
      ref = "owner/repo#pr/7",
    },
    dedup_key = "consensus:github-devloop/pr/owner/repo/7/review",
    attempt = 12,
    error = "review result failed\nwhile applying marker",
  }
end

return {
  test_dead_letter_delivery_logs_l2_triage_fact = function()
    local result = t.run_department("departments/dead_letter/main.lua", event(dead_payload()))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)

    local module = require("departments.dead_letter.main")
    local logs = capture_logs(function()
      module.pipeline(event(dead_payload()))
    end)
    t.eq(#logs, 1)
    t.is_true(logs[1]:find("github-devloop-pr dept=dead_letter tag=DEAD_LETTER", 1, true) ~= nil)
    t.is_true(logs[1]:find("source_ref=external:owner/repo#pr/7", 1, true) ~= nil)
    t.is_true(logs[1]:find("dedup_key=consensus:github-devloop/pr/owner/repo/7/review", 1, true) ~= nil)
  end,
}
