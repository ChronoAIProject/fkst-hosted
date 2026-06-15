local M = {}

function M.run(exec, cmd, timeout, context)
  local result = exec({ cmd = cmd, timeout = timeout })
  if type(result) ~= "table" or tonumber(result.exit_code) ~= 0 then
    error({
      class = "git-command-failed",
      result = result,
      message = "std.git: " .. tostring(context) .. " failed",
    })
  end
  return result
end

return M
