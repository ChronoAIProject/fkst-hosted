local env = require("workflow.env")

local M = {}

local allowed_env = {
  FKST_GITHUB_REPO = true,
  FKST_GITHUB_BOT_LOGIN = true,
  FKST_DEVLOOP_MANAGED_BOT_LOGINS = true,
  FKST_GITHUB_AUTHORIZED_LOGINS = true,
  FKST_GITHUB_WRITE = true,
  FKST_GITHUB_PROXY_POLL_LABEL_PREFIX = true,
  FKST_GITHUB_PROXY_REPLAY_BUDGET = true,
  FKST_DEBUG_STAMP = true,
}

local function read_env_command(name)
  if not allowed_env[name] then
    error("github-proxy: env-name-denied: env name is not allowed: " .. tostring(name))
  end
  return 'printf %s "$' .. name .. '"'
end

M.read_env_command = read_env_command
M.read_env = env.read_env(read_env_command, {
  missing_exec_error = "read_env requires exec_sync",
  propagate_exec_errors = true,
})

return M
