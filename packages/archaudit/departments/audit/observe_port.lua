local core = require("core")
local env_port = require("departments.audit.env_port")
local strings = require("contract.strings")

local M = {}

local allowed_env = {
  BIN = true,
  FKST_DURABLE_ROOT = true,
}

local read_env = env_port.read_env(allowed_env, {
  missing_exec_error = "archaudit: missing-exec: observe requires exec_sync",
  propagate_exec_errors = true,
})

local function decode_json(text)
  return json.decode(text)
end

function M.facts(exec)
  exec = exec or {}
  local run_sync = exec.exec_sync or exec_sync
  local run_argv = exec.exec_argv or exec_argv
  if type(run_sync) ~= "function" then
    error("archaudit: missing-exec: observe requires exec_sync")
  end
  if type(run_argv) ~= "function" then
    error("archaudit: missing-exec: observe requires exec_argv")
  end
  local bin = strings.trim(read_env("BIN", run_sync))
  if bin == "" then
    error("archaudit: observe-bin-unresolved: BIN is unset")
  end
  local durable_root = strings.trim(read_env("FKST_DURABLE_ROOT", run_sync))
  if durable_root == "" then
    error("archaudit: observe-durable-root-unresolved: FKST_DURABLE_ROOT is unset")
  end
  local ok_run, result = pcall(run_argv, { argv = { bin, "observe", "--durable-root", durable_root, "--json" }, timeout = 30 })
  if not ok_run then
    error("archaudit: observe-bin-unresolved: " .. tostring(result))
  end
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("archaudit: observe-unreadable: " .. tostring(result and result.stderr or "no result"))
  end
  local ok, decoded = pcall(decode_json, result.stdout or "")
  if not ok or type(decoded) ~= "table" then
    error("archaudit: observe-malformed-json: observe returned malformed JSON")
  end
  return core.validate_observe_facts(decoded)
end

return M
