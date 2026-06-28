local graph = require("testkit.graph")
local t = fkst.test

local edge_id = "github-proxy.github_issue_create_request -> github-proxy.github_issue_create"

local checker_fixture = [[
[
  {
    "consumer_dept": "github_issue_create",
    "consumer_pkg": "github-proxy",
    "edge_id": "github-proxy.github_issue_create_request -> github-proxy.github_issue_create",
    "owner_scope": "platform-owned",
    "producer_dept": "produce",
    "producer_pkg": "integration-coverage-producer",
    "queue": "github-proxy.github_issue_create_request",
    "status": "uncovered-allowlisted"
  }
]
]]

local function initial_event()
  return {
    queue = "integration-coverage-producer.integration_coverage_tick",
    payload = {
      raiser = "integration-coverage-producer.coverage_poll",
      slot = "2026-06-25T01:00:00Z",
    },
    source_ref = {
      kind = "cron",
      reference = "integration-coverage-producer.coverage_poll",
    },
  }
end

local function mock_env()
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', {
    stdout = "owner/repo",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_checker()
  t.mock_command("tools/check_repo_integration_coverage.py", {
    stdout = checker_fixture,
    stderr = "integration coverage check failed",
    exit_code = 1,
  })
end

local function mock_issue_reads()
  t.mock_command("gh issue list --repo owner/repo --state open --limit 100 --json 'number,title,state,labels'", {
    stdout = "[]",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(
    "gh issue list --repo owner/repo --state all --limit 100 --search 'coverage-edge-id: "
      .. edge_id
      .. "' --json 'number,title,state,author,body,labels,url'",
    {
      stdout = "[]",
      stderr = "",
      exit_code = 0,
    }
  )
end

return {
  test_run_graph_producer_raised_issue_create_request_reaches_github_proxy = function()
    mock_env()
    mock_checker()
    mock_issue_reads()

    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 4 }))
    graph.assert_covers(trace, {
      "github-proxy.github_issue_create_request -> github-proxy.github_issue_create",
    })

    local raised, _, raise_step_index = graph.require_raise(trace, "github-proxy.github_issue_create_request", function(item)
      local payload = item.payload or {}
      return payload.schema == "github-proxy.issue-create.v1"
        and payload.repo == "owner/repo"
        and payload.body:find("coverage-edge-id: " .. edge_id, 1, true) ~= nil
    end)

    local delivery, delivery_index = graph.require_delivery(trace, {
      queue = "github-proxy.github_issue_create_request",
      consumer = "github-proxy.github_issue_create",
    })
    t.eq(delivery.exit_code, 0)
    t.is_true(delivery_index > raise_step_index)
    t.eq(raised.queue, delivery.queue)
  end,
}
