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
helpers.mock_bot_env = function(...)
  if type(helpers.reset_pr_helper_state) == "function" then
    helpers.reset_pr_helper_state()
  end
  return base_mock_bot_env(...)
end

return helpers
