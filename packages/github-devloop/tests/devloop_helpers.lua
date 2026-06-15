local base = require("tests.devloop_base_helpers")
local pr = require("tests.devloop_pr_helpers")
local worktree = require("tests.devloop_worktree_helpers")
local entity_read_mocks = require("tests.entity_read_mock_helpers")

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

local function issue_identity_from_payload(payload)
  local entity = helpers.core.parse_entity_proposal_id(payload and payload.proposal_id)
  local source_ref = payload and payload.source_ref and payload.source_ref.ref
  local source_repo, source_issue = tostring(source_ref or ""):match("^(.+)#issue/(%d+)$")
  return source_repo or (entity and entity.repo) or "owner/repo",
    tonumber(source_issue) or (entity and entity.issue_number) or 42
end

local function mock_default_issue_claim(repo, number)
  local selected_repo = repo or "owner/repo"
  local selected_number = number or 42
  entity_read_mocks.mock_issue_read_forms(helpers.t, {
    repo = selected_repo,
    number = selected_number,
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  })
  entity_read_mocks.mock_issue_view_selector(helpers.t, {
    repo = selected_repo,
    number = selected_number,
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "assignees,author", 30)
end

local function mock_context_bundle(payload)
  local repo, issue_number = issue_identity_from_payload(payload)
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
  for _ = 1, 3 do
    helpers.t.mock_command("test -e", {
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
  entity_read_mocks.mock_issue_view_raw_selector(helpers.t, { repo = repo, number = issue_number }, "title,body,updatedAt,labels,comments,state", {
    stdout = bundle_json,
  })
  entity_read_mocks.mock_pr_view_raw_selector(helpers.t, {
    repo = repo,
    number = payload and payload.pr_number or 7,
  }, "title,body,headRefName,headRefOid,baseRefName,state,updatedAt,comments,labels", {
    stdout = pr_context_json,
  })
  helpers.t.mock_command("gh pr diff", {
    stdout = "diff --git a/file.lua b/file.lua\n+return true\n",
    stderr = "",
    exit_code = 0,
  })
  entity_read_mocks.mock_issue_board_digest_list(helpers.t, repo, {})
  entity_read_mocks.mock_issue_list_command(helpers.t, helpers.core.gh_issue_list_recent_closed_cmd(repo, 30), {})
  for _ = 1, 12 do
    helpers.t.mock_command("touch ", ok)
  end
  for _ = 1, 12 do
    helpers.t.mock_command("printf %s '", ok)
    helpers.t.mock_command(" > ", ok)
  end
  for _ = 1, 3 do
    helpers.t.mock_command("python3 -c", ok)
  end
  for _ = 1, 12 do
    helpers.t.mock_command("test -r", ok)
  end
  for _ = 1, 12 do
    helpers.t.mock_command("wc -c < ", {
      stdout = "1\n",
      stderr = "",
      exit_code = 0,
    })
  end
end

helpers.run_observe = function(...)
  mock_empty_dependencies()
  local event = ...
  mock_context_bundle(event)
  return base_run_observe(...)
end

helpers.run_result = function(...)
  mock_empty_dependencies()
  return base_run_result(...)
end

helpers.run_implement = function(...)
  mock_empty_dependencies()
  local payload = ...
  mock_context_bundle(payload)
  return base_run_implement(...)
end

for _, name in ipairs({
  "run_loop",
  "run_review_pr",
  "run_review_loop",
  "run_fix",
  "run_review_meta",
  "run_review_reconcile",
  "run_fix_reconcile",
  "run_decompose",
}) do
  local base_run = helpers[name]
  helpers[name] = function(...)
    local payload = ...
    local repo, issue_number = issue_identity_from_payload(payload)
    mock_context_bundle(payload)
    mock_default_issue_claim(repo, issue_number)
    return base_run(...)
  end
end

local base_run_observe_pr = helpers.run_observe_pr
helpers.run_observe_pr = function(...)
  mock_default_issue_claim()
  return base_run_observe_pr(...)
end

local base_run_review_result = helpers.run_review_result
helpers.run_review_result = function(...)
  mock_default_issue_claim()
  return base_run_review_result(...)
end

local base_run_merge = helpers.run_merge
helpers.run_merge = function(payload, ...)
  local entity = helpers.core.parse_entity_proposal_id(payload and payload.proposal_id)
  if entity ~= nil and entity.issue_number ~= nil then
    mock_default_issue_claim()
  end
  return base_run_merge(payload, ...)
end

helpers.mock_bot_env = function(...)
  if type(helpers.reset_pr_helper_state) == "function" then
    helpers.reset_pr_helper_state()
  end
  return base_mock_bot_env(...)
end

helpers.mock_context_bundle = mock_context_bundle
helpers.mock_default_issue_claim = mock_default_issue_claim
helpers.issue_identity_from_payload = issue_identity_from_payload
return helpers
