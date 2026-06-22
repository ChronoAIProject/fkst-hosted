local S = {}

local github_handle = nil
local git_handle = nil

function S.github(run)
  if run ~= nil then
    if type(run) ~= "function" then
      error("github-devloop: GitHub adapter requires an exec function")
    end
    return require("forge.github").new(run)
  end
  if github_handle == nil then
    if type(exec_argv) ~= "function" then
      error("github-devloop: GitHub adapter requires exec_argv")
    end
    github_handle = require("forge.github").new(exec_argv)
  end
  return github_handle
end

function S.gh_result(fn)
  local ok, result_or_error = pcall(fn)
  if ok then
    return result_or_error
  end
  if type(result_or_error) == "table" and result_or_error.result ~= nil then
    return result_or_error.result
  end
  error(result_or_error)
end

function S.git()
  if git_handle == nil then
    if type(exec_argv) ~= "function" then
      error("github-devloop: git adapter requires exec_argv")
    end
    git_handle = require("forge.git").new(exec_argv)
  end
  return git_handle
end

function S.install()
end

return S
