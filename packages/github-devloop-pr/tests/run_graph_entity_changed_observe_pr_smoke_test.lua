local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local h = require("tests.devloop_helpers")
local graph = require("testkit.graph")
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local t = h.t
local core = h.core

local repo = "owner/repo"
local pr_number = 7

local function observe_spec()
  return require("departments.observe_pr.main").spec
end

local function source_ref()
  return entity_lib.pr_source_ref(repo, pr_number)
end

local function initial_event()
  return {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "pr",
      repo = repo,
      number = pr_number,
      title = "Run graph PR",
      updated_at = "2026-06-03T02:03:04Z",
      dedup_key = "owner/repo#pr/7@2026-06-03T02:03:04Z",
      source_ref = source_ref(),
    },
    source_ref = {
      kind = "external",
      reference = "owner/repo#pr/7",
    },
  }
end

local function mock_runtime_and_pr_view()
  for _ = 1, 4 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_BOT_LOGIN"), {
      stdout = "fkst-test-bot",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_UPSTREAM_BRANCH"), {
      stdout = "dev",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_INTEGRATION_BRANCH"), {
      stdout = "dev",
      stderr = "",
      exit_code = 0,
    })
  end

  entity_read_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = pr_number,
    head = "feature/unmanaged",
    head_sha = "def456",
    base_branch = "dev",
    comments = {},
    state = "OPEN",
    updated_at = "2026-06-03T02:03:04Z",
  }, entity_read_mocks.pr_origin_selector)
end

return {
  test_run_graph_entity_changed_delivers_to_observe_pr = function()
    mock_runtime_and_pr_view()

    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 3 }))
    graph.assert_covers(trace, {
      "github-proxy.github_entity_changed -> github-devloop-pr.observe_pr",
    })

    local step = graph.require_delivery(trace, {
      queue = "github-proxy.github_entity_changed",
      consumer = "github-devloop-pr.observe_pr",
    })
    t.eq(step.exit_code, 0)
    t.is_true(observe_spec().consumes[1] == "github-proxy.github_entity_changed")
  end,
}
