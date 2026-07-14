local strings = require("contract.strings")

local M = {}

M.max_reply_len = 2000

function M.is_verdict(value)
  return value == "approve"
    or value == "comment"
    or value == "reject"
    or value == "abstain"
end

function M.is_valid(result)
  return type(result) == "table"
    and result.exit_code == 0
    and M.is_verdict(result.verdict)
    and strings.is_bounded_string(result.reply, M.max_reply_len)
end

local function counts(results)
  local value = {
    total = 0,
    valid = 0,
    worker_failures = 0,
    protocol_failures = 0,
  }
  if type(results) ~= "table" then
    return value
  end
  for _, result in ipairs(results) do
    value.total = value.total + 1
    if type(result) ~= "table" or result.exit_code ~= 0 then
      value.worker_failures = value.worker_failures + 1
      if value.worker_exit_code == nil and type(result) == "table" then
        value.worker_exit_code = result.exit_code
        value.worker_stderr = result.stderr
      end
    elseif M.is_valid(result) then
      value.valid = value.valid + 1
    else
      value.protocol_failures = value.protocol_failures + 1
    end
  end
  return value
end

local function failure_detail(value, phase)
  local detail = "phase=" .. tostring(phase)
    .. " valid_answers=" .. tostring(value.valid)
    .. " worker_failures=" .. tostring(value.worker_failures)
    .. " protocol_failures=" .. tostring(value.protocol_failures)
  if value.worker_exit_code ~= nil then
    detail = detail .. " worker_exit_code=" .. tostring(value.worker_exit_code)
  end
  if type(value.worker_stderr) == "string" and value.worker_stderr ~= "" then
    detail = detail .. " stderr=" .. value.worker_stderr
  end
  return detail
end

local function raise_failure(value, phase)
  local detail = failure_detail(value, phase)
  if value.worker_failures > 0 then
    error("consensus: codex-failed: " .. detail, 0)
  end
  if value.protocol_failures > 0 then
    error("consensus: angle-output-unparseable: " .. detail, 0)
  end
  error("consensus: angle-results-unavailable: " .. detail, 0)
end

function M.assert_all_valid(results, phase)
  local value = counts(results)
  if value.total == 0 or value.valid ~= value.total then
    raise_failure(value, phase)
  end
end

return M
