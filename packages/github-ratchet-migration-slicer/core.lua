local env = require("workflow.env")
local error_facts = require("contract.error_facts")
local logging = require("workflow.logging")
local strings = require("contract.strings")
local forge_strings = require("forge.strings")

local M = {}


local allowed_env = {
  FKST_GITHUB_BOT_LOGIN = true,
  FKST_GITHUB_REPO = true,
  FKST_GITHUB_WRITE = true,
  FKST_DEVLOOP_MANAGED_BOT_LOGINS = true,
}

local function read_env_command(name)
  if not allowed_env[name] then
    error("github-ratchet-migration-slicer: env-not-allowed: " .. tostring(name))
  end
  return 'printf %s "$' .. name .. '"'
end

function M.read_env_command(name)
  return read_env_command(name)
end

M.read_env = env.read_env(read_env_command)

M.strip_bot_login_suffix = forge_strings.strip_bot_login_suffix

function M.iso_timestamp_epoch_seconds(timestamp)
  local year, month, day, hour, minute, second = tostring(timestamp or ""):match(
    "^(%d%d%d%d)%-(%d%d)%-(%d%d)T(%d%d)[%-:](%d%d)[%-:](%d%d)Z$"
  )
  if year == nil then
    return nil
  end
  year = tonumber(year)
  month = tonumber(month)
  day = tonumber(day)
  hour = tonumber(hour)
  minute = tonumber(minute)
  second = tonumber(second)
  if month < 1 or month > 12
    or day < 1 or day > 31
    or hour > 23
    or minute > 59
    or second > 59 then
    return nil
  end

  local adjusted_year = year
  local adjusted_month = month
  if adjusted_month <= 2 then
    adjusted_year = adjusted_year - 1
    adjusted_month = adjusted_month + 12
  end
  local era = math.floor(adjusted_year / 400)
  local year_of_era = adjusted_year - era * 400
  local day_of_year = math.floor((153 * (adjusted_month - 3) + 2) / 5) + day - 1
  local day_of_era = year_of_era * 365
    + math.floor(year_of_era / 4)
    - math.floor(year_of_era / 100)
    + day_of_year
  local days_since_epoch = era * 146097 + day_of_era - 719468
  return days_since_epoch * 86400 + hour * 3600 + minute * 60 + second
end

M.error_fingerprint = error_facts.error_fingerprint

function M.error_class_from_message(message)
  local text = tostring(message or "")
  local class = text:match("github%-ratchet%-migration%-slicer: ([%w%-]+):")
    or text:match("github%-ratchet%-migration%-slicer: ([%w%-]+) failed:")
  return class or "caught-failure"
end

function M.log_error_fact(level, dept, proposal_id, tag, error_class, queue, message, context)
  local fields = error_facts.error_fact_fields(error_class, queue, dept, message, context)
  table.insert(fields, "queue=" .. error_facts.one_line(queue))
  table.insert(fields, "error=" .. error_facts.one_line(message))
  M.log_line(level or "error", dept, proposal_id, tag or "FAILURE", fields)
end

local event_source_ref = error_facts.event_source_ref

function M.wrap_pipeline_failure(dept, fn)
  return function(event)
    local ok, err = pcall(fn, event)
    if ok then
      return err
    end
    M.log_error_fact("error", dept, "ratchet-migration", "FAILURE", M.error_class_from_message(err), type(event) == "table" and event.queue or nil, err, {
      source_ref = event_source_ref(event),
      attempt = type(event) == "table" and event.attempt or nil,
    })
    error(err, 0)
  end
end

function M.log_line(level, dept, proposal_id, tag, fields)
  return logging.log_line("github-ratchet-migration-slicer", level, dept, proposal_id, tag, fields)
end

function M.log_entry(dept, event, proposal_id, dedup_key)
  return logging.log_entry("github-ratchet-migration-slicer", dept, event, proposal_id, dedup_key)
end

M._trim = strings.trim

function M.ratchet_slice_ledger_ref(entry_key)
  return "refs/fkst/migration-slices/" .. tostring(entry_key)
end

function M.parse_ratchet_slice_ledger_ref_sha(stdout)
  local sha = tostring(stdout or ""):match("^(%x+)%s+refs/")
  if sha ~= nil and #sha == 40 then
    return sha
  end
  return nil
end

function M.ratchet_slice_ledger_message(stdout)
  local text = tostring(stdout or "")
  local _, finish = text:find("\n\n", 1, true)
  if finish == nil then
    return text
  end
  return text:sub(finish + 1)
end

function M.decode_ratchet_slice_ledger(stdout)
  local ok, decoded = pcall(json.decode, M.ratchet_slice_ledger_message(stdout))
  if not ok or type(decoded) ~= "table" then
    return nil
  end
  return decoded
end

return M
