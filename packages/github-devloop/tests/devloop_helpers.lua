local base = require("tests.devloop_base_helpers")
local pr = require("tests.devloop_pr_helpers")
local worktree = require("tests.devloop_worktree_helpers")

local helpers = {}
for key, value in pairs(base) do
  helpers[key] = value
end
for key, value in pairs(pr) do
  helpers[key] = value
end
for key, value in pairs(worktree) do
  helpers[key] = value
end

local base_mock_bot_env = helpers.mock_bot_env
local base_run_observe = helpers.run_observe
local base_run_result = helpers.run_result
local base_run_implement = helpers.run_implement

local function mock_empty_dependencies()
  helpers.t.mock_command("gh api graphql", {
    stdout = '{"data":{"repository":{"issue":{"blockedBy":{"nodes":[]}}}}}\n',
    stderr = "",
    exit_code = 0,
  })
end

helpers.run_observe = function(...)
  mock_empty_dependencies()
  return base_run_observe(...)
end

helpers.run_result = function(...)
  mock_empty_dependencies()
  return base_run_result(...)
end

helpers.run_implement = function(...)
  mock_empty_dependencies()
  return base_run_implement(...)
end

helpers.mock_bot_env = function(...)
  if type(helpers.reset_pr_helper_state) == "function" then
    helpers.reset_pr_helper_state()
  end
  return base_mock_bot_env(...)
end

return helpers
