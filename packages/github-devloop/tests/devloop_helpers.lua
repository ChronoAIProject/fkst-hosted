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
local bundle_json = '{"title":"Implement decision recorder","body":"Full issue body","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[{"name":"fkst-dev:enabled"}],"comments":[]}\n'
local pr_context_json = '{"title":"PR title","body":"PR body","headRefName":"devloop-owner-repo-42-01HY","headRefOid":"def456","baseRefName":"dev","state":"OPEN","updatedAt":"2026-06-04T01:02:03Z","comments":[],"labels":[]}\n'

local function mock_empty_dependencies()
  helpers.t.mock_command("gh api graphql", {
    stdout = '{"data":{"repository":{"issue":{"blockedBy":{"nodes":[]}}}}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_context_bundle()
  local ok = { stdout = "", stderr = "", exit_code = 0 }
  for _ = 1, 8 do
    helpers.t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, 3 do
    helpers.t.mock_command("test -d", {
      stdout = "",
      stderr = "",
      exit_code = 1,
    })
  end
  helpers.t.mock_command("install -d -m 0755", ok)
  helpers.t.mock_command("mktemp -d", {
    stdout = "/tmp/fkst-packages-test/github-devloop/runtime/context/.bundle-tmp.mocked\n",
    stderr = "",
    exit_code = 0,
  })
  helpers.t.mock_command("--json title,body,updatedAt,labels,comments,state", {
    stdout = bundle_json,
    stderr = "",
    exit_code = 0,
  })
  helpers.t.mock_command("--json title,body,headRefName,headRefOid,baseRefName,state,updatedAt,comments,labels", {
    stdout = pr_context_json,
    stderr = "",
    exit_code = 0,
  })
  helpers.t.mock_command("gh pr diff", {
    stdout = "diff --git a/file.lua b/file.lua\n+return true\n",
    stderr = "",
    exit_code = 0,
  })
  helpers.t.mock_command("--state open --limit 100 --json number,title,labels", {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
  helpers.t.mock_command("--state closed --limit 30 --json number,title,closedAt,labels", {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
  for _ = 1, 5 do
    helpers.t.mock_command(" > ", ok)
  end
  helpers.t.mock_command("python3 -c", ok)
  for _ = 1, 8 do
    helpers.t.mock_command("test -r", ok)
  end
  for _ = 1, 8 do
    helpers.t.mock_command("wc -c < ", {
      stdout = "1\n",
      stderr = "",
      exit_code = 0,
    })
  end
end

helpers.run_observe = function(...)
  mock_empty_dependencies()
  mock_context_bundle()
  return base_run_observe(...)
end

helpers.run_result = function(...)
  mock_empty_dependencies()
  return base_run_result(...)
end

helpers.run_implement = function(...)
  mock_empty_dependencies()
  mock_context_bundle()
  return base_run_implement(...)
end

for _, name in ipairs({
  "run_loop",
  "run_review_pr",
  "run_review_loop",
  "run_fix",
  "run_review_meta",
  "run_decompose",
}) do
  local base_run = helpers[name]
  helpers[name] = function(...)
    mock_context_bundle()
    return base_run(...)
  end
end

helpers.mock_bot_env = function(...)
  if type(helpers.reset_pr_helper_state) == "function" then
    helpers.reset_pr_helper_state()
  end
  return base_mock_bot_env(...)
end

helpers.mock_context_bundle = mock_context_bundle
return helpers
