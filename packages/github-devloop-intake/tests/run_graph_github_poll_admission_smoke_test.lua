local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local base_ids = require("devloop.base_ids")
local graph = require("testkit.graph")
local t = fkst.test
local core = require("core")
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local repo = "owner/repo"
local issue_number = 42

local function source_ref()
  return entity_lib.issue_source_ref(repo, issue_number)
end

local function mock_env()
  for _ = 1, 8 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_REPO"), { stdout = repo, stderr = "", exit_code = 0 })
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_BOT_LOGIN"), { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_WRITE"), { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_CLAIM_MODE"), { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_PROXY_POLL_LABEL_PREFIX"', { stdout = "fkst-dev:,fkst-class:", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_PROXY_REPLAY_BUDGET"', { stdout = "1", stderr = "", exit_code = 0 })
  end
  for _ = 1, 3 do
    t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_FORK_GRACE_HOURS"), { stdout = "", stderr = "", exit_code = 0 })
  end
end

local function mock_proxy_poll_lists()
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/issues?state=open&per_page=100'", {
    stdout = '[[{"number":42,"title":"Fresh unmanaged issue","html_url":"https://github.example/owner/repo/issues/42","updated_at":"2026-06-03T01:02:03Z","state":"open","labels":[{"name":"bug"}]}]]\n',
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&per_page=100'", {
    stdout = "[[]]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_admission_issue_view()
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = repo,
    number = issue_number,
    title = "Fresh unmanaged issue",
    body = "",
    updated_at = "2026-06-03T01:02:03Z",
    state = "OPEN",
    labels = { "bug" },
    comments = {},
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "title,body,createdAt,updatedAt,labels,comments,state,assignees,author")
end

return {
  test_run_graph_github_poll_reaches_intake_admission_candidate_without_intake_poll = function()
    mock_env()
    mock_proxy_poll_lists()
    mock_admission_issue_view()

    local trace = graph.require_quiescent(graph.run("github-proxy.github_poll", { max_steps = 4 }))
    graph.assert_covers(trace, {
      "github-proxy.github_poll_tick -> github-proxy.github_poll",
      "github-proxy.github_entity_changed -> github-devloop-intake.admission",
    })

    local spec = require("departments.admission.main").spec
    t.eq(spec.consumes[1], "github-proxy.github_entity_changed")
    t.eq(spec.produces[1], "devloop_intake_candidate")

    local _, admission_index = graph.require_delivery(trace, {
      queue = "github-proxy.github_entity_changed",
      consumer = "github-devloop-intake.admission",
    })
    local raised, _, raised_step_index = graph.require_raise(trace, "github-devloop-intake.devloop_intake_candidate", function(item)
      local payload = item.payload or {}
      return payload.schema == "github-devloop.intake-candidate.v1"
        and payload.repo == repo
        and tostring(payload.issue_number) == tostring(issue_number)
        and payload.source_ref ~= nil
        and payload.source_ref.ref == source_ref().ref
    end)
    t.eq(raised_step_index, admission_index)
    t.eq(raised.payload.proposal_id, base_ids.proposal_id(repo, issue_number))

    for _, step in ipairs(trace.steps or {}) do
      t.is_true(step.consumer ~= "github-devloop-intake.intake_scan")
      t.is_true(step.consumer ~= "github-devloop-intake.intake_probe")
    end
  end,
}
