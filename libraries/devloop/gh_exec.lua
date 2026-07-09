local content_filter = require("forge.github.content_filter")
local stdout_policy = require("forge.github.stdout_policy")

local G = {}

local gh_program = table.concat({ "g", "h" })

local function gh_exec_opts(cmd_or_opts, timeout)
  local opts = {}
  if type(cmd_or_opts) == "table" then
    for key, value in pairs(cmd_or_opts) do
      opts[key] = value
    end
  else
    opts.cmd = cmd_or_opts
  end
  opts.timeout = opts.timeout or timeout or 30
  return opts
end

local function normalize_gh_argv_exec_opts(cmd_or_opts, timeout)
  local opts = gh_exec_opts(cmd_or_opts, timeout)
  if type(opts.argv) ~= "table" or opts.argv[1] ~= gh_program then
    error("github-devloop: GitHub exec requires GitHub argv")
  end
  return {
    argv = opts.argv,
    timeout = opts.timeout,
    stdout_policy = opts.stdout_policy,
  }
end

local function filter_stdout(result, policy, author_policy)
  return content_filter.apply_gh_content_filter(result, nil, policy, author_policy, stdout_policy)
end

function G.gh_exec(cmd_or_opts, timeout, exec, policy, author_policy)
  local run = exec or exec_argv
  if type(run) ~= "function" then
    error("github-devloop: GitHub exec requires exec_argv")
  end
  local opts = normalize_gh_argv_exec_opts(cmd_or_opts, timeout)
  local effective_policy = policy or opts.stdout_policy
  stdout_policy.validate(effective_policy)
  return filter_stdout(run({ argv = opts.argv, timeout = opts.timeout }), effective_policy, author_policy)
end

return G
