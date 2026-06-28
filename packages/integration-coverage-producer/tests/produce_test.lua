local testing = require("testkit.testing")
local github_fake = require("forge.github_fake")
local produce = require("departments.produce.main")
local t = fkst.test

local checker_fixture = [[
[
  {
    "consumer_dept": "outbound_glue",
    "consumer_pkg": "github-autochrono",
    "edge_id": "autochrono.reply -> github-autochrono.outbound_glue",
    "owner_scope": "platform-owned",
    "producer_dept": "reply",
    "producer_pkg": "autochrono",
    "queue": "autochrono.reply",
    "status": "uncovered-allowlisted"
  },
  {
    "consumer_dept": "review_result",
    "consumer_pkg": "github-devloop-pr",
    "edge_id": "consensus.consensus_reached -> github-devloop-pr.review_result",
    "owner_scope": "platform-owned",
    "producer_dept": "decide",
    "producer_pkg": "consensus",
    "queue": "consensus.consensus_reached",
    "status": "uncovered-allowlisted"
  }
]
]]

local covered_fixture = [[
[
  {
    "consumer_dept": "propose",
    "consumer_pkg": "autochrono",
    "edge_id": "autochrono.issue -> autochrono.propose",
    "owner_scope": "platform-owned",
    "producer_dept": "inbound_glue",
    "producer_pkg": "github-autochrono",
    "queue": "autochrono.issue",
    "status": "covered"
  }
]
]]

local function tick_event()
  return {
    queue = "integration-coverage-producer.integration_coverage_tick",
    payload = {
      raiser = "integration-coverage-producer.coverage_poll",
      slot = "2026-06-25T01:00:00Z",
    },
  }
end

local function mock_env()
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', {
    stdout = "owner/repo",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_checker(stdout, exit_code)
  t.mock_command("tools/check_repo_integration_coverage.py", {
    stdout = stdout,
    stderr = exit_code == 0 and "" or "integration coverage check failed",
    exit_code = exit_code or 0,
  })
end

local function fake_department(open_issues_stdout, issue_search_stdout)
  local model = github_fake.model()
  local github = github_fake.new(model)
  local lists = {}
  local searches = {}
  function github.issue_list_cli(repo, state, limit, fields, timeout)
    table.insert(lists, { repo = repo, state = state, limit = limit, fields = fields, timeout = timeout })
    return {
      stdout = open_issues_stdout or "[]",
      stderr = "",
      exit_code = 0,
    }
  end
  function github.issue_search(repo, query, fields, timeout)
    table.insert(searches, { repo = repo, query = query, fields = fields, timeout = timeout })
    return {
      stdout = issue_search_stdout or "[]",
      stderr = "",
      exit_code = 0,
    }
  end
  local dept = produce.make_department({ github = github })
  dept.model = model
  dept.lists = lists
  dept.searches = searches
  return dept
end

local function run(dept)
  return testing.run_fake(dept, tick_event())
end

local function first_request(result)
  t.eq(#result.raises, 1)
  t.eq(result.raises[1].queue, "github-proxy.github_issue_create_request")
  return result.raises[1].payload
end

local function with_file_exists(existing, fn)
  local old_file = file
  file = {
    exists = function(path)
      return existing[path] == true
    end,
    read = function(path)
      if existing[path] == true then
        return ""
      end
      error("missing file: " .. tostring(path), 0)
    end,
  }
  local ok, result = pcall(fn)
  file = old_file
  if not ok then
    error(result, 0)
  end
  return result
end

local function with_coverage_substrate(fn)
  return with_file_exists({
    ["migration/integration-edge-coverage.allowlist"] = true,
  }, fn)
end

local function checker_call()
  for _, call in ipairs(t.command_calls()) do
    if call.program == "python3" and tostring(call.args[1] or ""):find("tools/check_repo_integration_coverage.py", 1, true) ~= nil then
      return call
    end
  end
  return nil
end

return {
  test_uncovered_edges_on_idle_board_produces_one_coverage_issue = function()
    mock_env()
    mock_checker(checker_fixture, 1)
    local dept = fake_department("[]")

    local result = with_coverage_substrate(function()
      return run(dept)
    end)
    local request = first_request(result)
    local call = checker_call()

    t.eq(request.schema, "github-proxy.issue-create.v1")
    t.eq(request.repo, "owner/repo")
    t.eq(request.title, "test: run_graph coverage for autochrono.reply -> github-autochrono.outbound_glue")
    t.is_true(request.body:find("coverage-edge-id: autochrono.reply -> github-autochrono.outbound_glue", 1, true) ~= nil)
    t.is_true(request.body:find("graph.assert_covers", 1, true) ~= nil)
    t.is_true(request.body:find("Remove the allowlist line", 1, true) ~= nil)
    t.is_true(request.body:find("python3 scripts/check_repo.py", 1, true) ~= nil)
    t.is_true(request.dedup_key:find("integration-coverage/owner/repo/", 1, true) == 1)
    t.eq(request.source_ref.kind, "repo-site")
    t.eq(#dept.lists, 1)
    t.eq(dept.lists[1].state, "open")
    t.eq(dept.lists[1].fields, "number,title,state,labels")
    t.eq(#dept.searches, 1)
    t.is_true(dept.searches[1].query:find("coverage-edge-id: autochrono.reply -> github-autochrono.outbound_glue", 1, true) ~= nil)
    t.eq(call.program, "python3")
    t.is_true(call.args[1]:find("packages/integration-coverage-producer/tools/check_repo_integration_coverage.py", 1, true) ~= nil)
    t.eq(call.args[2], "--json")
    t.eq(call.args[3], nil)
  end,

  test_existing_open_coverage_issue_skips_edge_and_files_next_one = function()
    mock_env()
    mock_checker(checker_fixture, 1)
    local existing = '[{"number":1491,"state":"OPEN","body":"coverage-edge-id: autochrono.reply -> github-autochrono.outbound_glue","author":{"login":"fkst-test-bot"}}]'
    local search_count = 0
    local model = github_fake.model()
    local github = github_fake.new(model)
    function github.issue_list_cli(_repo, _state, _limit, _fields, _timeout)
      return { stdout = "[]", stderr = "", exit_code = 0 }
    end
    function github.issue_search(_repo, query, _fields, _timeout)
      search_count = search_count + 1
      if query:find("autochrono.reply", 1, true) ~= nil then
        return { stdout = existing, stderr = "", exit_code = 0 }
      end
      return { stdout = "[]", stderr = "", exit_code = 0 }
    end
    local dept = produce.make_department({ github = github })

    local result = with_coverage_substrate(function()
      return run(dept)
    end)
    local request = first_request(result)

    t.eq(request.title, "test: run_graph coverage for consensus.consensus_reached -> github-devloop-pr.review_result")
    t.is_true(request.body:find("coverage-edge-id: consensus.consensus_reached -> github-devloop-pr.review_result", 1, true) ~= nil)
    t.eq(search_count, 2)
  end,

  test_busy_board_skips_without_issue = function()
    mock_env()
    mock_checker(checker_fixture, 1)
    local busy = [[
[
  {"number":1,"title":"busy 1","state":"OPEN","labels":[{"name":"fkst-dev:ready"}]},
  {"number":2,"title":"busy 2","state":"OPEN","labels":[{"name":"fkst-dev:thinking"}]},
  {"number":3,"title":"busy 3","state":"OPEN","labels":[{"name":"fkst-dev:reviewing"}]},
  {"number":4,"title":"busy 4","state":"OPEN","labels":[{"name":"fkst-dev:fixing"}]}
]
]]
    local dept = fake_department(busy, "[]")

    local result = with_coverage_substrate(function()
      return run(dept)
    end)

    t.eq(#result.raises, 0)
    t.eq(#dept.lists, 1)
    t.eq(#dept.searches, 0)
  end,

  test_no_uncovered_edges_skips_without_issue = function()
    mock_env()
    mock_checker(covered_fixture, 0)
    local dept = fake_department("[]", "[]")

    local result = with_coverage_substrate(function()
      return run(dept)
    end)

    t.eq(#result.raises, 0)
    t.eq(#dept.searches, 0)
  end,

  test_missing_coverage_substrate_noops_without_checker_or_github = function()
    local dept = fake_department("[]", "[]")

    local result = with_file_exists({}, function()
      return run(dept)
    end)

    t.eq(#result.raises, 0)
    t.eq(#dept.lists, 0)
    t.eq(#dept.searches, 0)
    t.eq(#t.command_calls(), 0)
  end,
}
