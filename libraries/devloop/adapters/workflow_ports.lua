local M = {}

function M.from_devloop(devloop)
  if type(devloop) ~= "table" then
    error("devloop.adapters.workflow_ports: missing devloop table")
  end
  local trusted_bot_login = devloop.trusted_bot_login
  if type(trusted_bot_login) ~= "function" then
    error("devloop.adapters.workflow_ports: missing trusted_bot_login")
  end
  return {
    trusted_bot_login = function(...)
      return trusted_bot_login(...)
    end,
  }
end

return M
