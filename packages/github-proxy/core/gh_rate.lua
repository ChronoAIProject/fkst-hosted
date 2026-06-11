local S = {}

function S.install(M)
local function command_result_exit_code(result)
  if type(result) ~= "table" then
    return nil
  end
  return tonumber(result.exit_code)
end

function M.gh_rate_pool()
  return { name = "gh" }
end

function M.gh_exec_opts(cmd_or_opts, timeout)
  local opts = {}
  if type(cmd_or_opts) == "table" then
    for key, value in pairs(cmd_or_opts) do
      opts[key] = value
    end
  else
    opts.cmd = cmd_or_opts
  end
  opts.timeout = opts.timeout or timeout or 30
  opts.rate_pool = M.gh_rate_pool()
  return opts
end

function M.gh_exec_result(cmd, timeout, context, exec)
  local run = exec or exec_sync
  local result = run(M.gh_exec_opts(cmd, timeout))
  if command_result_exit_code(result) ~= 0 then
    return false, M.gh_error(context or "gh command", result)
  end
  return true, result
end

function M.gh_exec(cmd, timeout, context, exec)
  local ok, result_or_error = M.gh_exec_result(cmd, timeout, context, exec)
  if not ok then
    error(result_or_error.message)
  end
  return result_or_error
end
end

return S
