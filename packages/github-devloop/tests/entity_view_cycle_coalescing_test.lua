local h = require("tests.devloop_helpers")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local codex_status = require("tests.codex_status_helpers")
local devloop_base = require("devloop.base")
local m_builders = require("devloop.markers.builders")
local t = h.t
local core = h.core

local repo = "owner/repo"
local issue_number = 42
local updated_at = "2026-06-03T01:02:03Z"
local proposal_id = "github-devloop/issue/owner/repo/42"
local version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"

local function proposal_for(number)
  return "github-devloop/issue/" .. repo .. "/" .. tostring(number)
end

local function pr_proposal_for(number)
  return "github-devloop/pr/" .. repo .. "/" .. tostring(number)
end

local function run_liveness_scan(run_opts)
  return t.run_department("departments/liveness_scan/main.lua", {
    queue = "devloop_liveness_tick",
    payload = {
      schema = "github-devloop.tick.v1",
    },
    ts = "2026-06-03T01:32:03Z",
  }, run_opts)
end

local function run_observe_issue(payload, run_opts)
  return t.run_department("departments/observe_issue/main.lua", {
    queue = "devloop_observe_issue",
    payload = payload,
    ts = "2026-06-03T01:32:04Z",
  }, run_opts)
end

local function comments_rest_command(number)
  return h.argv_rendered("gh api --paginate --slurp repos/" .. repo .. "/issues/" .. tostring(number or issue_number) .. "/comments?per_page=100")
end

local function count_comment_stream_reads(number)
  local expected = number ~= nil and comments_rest_command(number) or nil
  local count = 0
  for _, call in ipairs(t.command_calls()) do
    local rendered = h.argv_rendered(tostring(call.rendered or ""))
    if expected ~= nil and rendered == expected then
      count = count + 1
    elseif expected == nil
      and rendered:find("^gh api %-%-paginate %-%-slurp repos/" .. repo .. "/issues/%d+/comments%?per_page=100$") ~= nil then
      count = count + 1
    end
  end
  return count
end

local function mock_repo_env()
  t.mock_command(devloop_base.read_env_command("FKST_GITHUB_REPO"), {
    stdout = repo,
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_list()
  t.mock_command(core.gh_issue_list_observe_cmd(repo), {
    stdout = '[{"number":42,"state":"open","updated_at":"' .. updated_at .. '"}]\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_list_numbers(numbers)
  local items = {}
  for _, number in ipairs(numbers) do
    table.insert(items, '{"number":' .. tostring(number) .. ',"state":"open","updated_at":"' .. updated_at .. '"}')
  end
  t.mock_command(core.gh_issue_list_observe_cmd(repo), {
    stdout = "[" .. table.concat(items, ",") .. "]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_state()
  entity_read_mocks.mock_issue_read_forms(t, {
    repo = repo,
    number = issue_number,
    title = "Coalesced issue",
    state = "OPEN",
    updated_at = updated_at,
    labels = { "fkst-dev:enabled", "fkst-dev:thinking" },
    comments = {
      {
        body = core.state_marker(proposal_id, "thinking", version),
        author_login = "fkst-test-bot",
        created_at = "2026-06-03T01:00:00Z",
      },
    },
    assignees = { "fkst-test-bot" },
    times = 2,
  })
end

return {
  test_liveness_cycle_over_k_entities_bounds_comment_stream_reads_across_departments = function()
    local run_opts = h.opts("entity-view-cycle-coalescing-k")
    local numbers = { 42, 43, 44 }
    local delegated_issue_number = 44
    local delegated_pr_number = 144
    mock_repo_env()
    mock_issue_list_numbers(numbers)
    for _, number in ipairs(numbers) do
      local issue_proposal = proposal_for(number)
      local state = number == delegated_issue_number and "awaiting-pr" or "thinking"
      local issue_comments = {
        {
          body = core.state_marker(issue_proposal, state, version .. "-" .. tostring(number)),
          author_login = "fkst-test-bot",
          created_at = "2026-06-03T01:00:00Z",
        },
      }
      if number == delegated_issue_number then
        table.insert(issue_comments, {
          body = m_builders.pr_delegation_marker(
            issue_proposal,
            pr_proposal_for(delegated_pr_number),
            delegated_pr_number,
            version .. "-" .. tostring(number),
            "g1"
          ),
          author_login = "fkst-test-bot",
          created_at = "2026-06-03T01:00:01Z",
        })
      end
      entity_read_mocks.mock_issue_read_forms(t, {
        repo = repo,
        number = number,
        title = "Coalesced issue " .. tostring(number),
        state = "OPEN",
        updated_at = updated_at,
        labels = { "fkst-dev:enabled", core.state_label(state) },
        comments = issue_comments,
        assignees = { "fkst-test-bot" },
        times = 2,
      })
      codex_status.seed_role_codex_run(run_opts, "consensus", issue_proposal, version .. "-" .. tostring(number))
    end
    entity_read_mocks.mock_pr_read_forms(t, {
      repo = repo,
      number = delegated_pr_number,
      head = "devloop-owner-repo-44-01HY",
      head_sha = "def456",
      base_branch = "dev",
      state = "OPEN",
      updated_at = updated_at,
      comments = {
        {
          body = m_builders.pr_origin_marker(proposal_for(delegated_issue_number), delegated_issue_number, "devloop-owner-repo-44-01HY", version .. "-" .. tostring(delegated_issue_number), "dev")
            .. "\n" .. core.state_marker(pr_proposal_for(delegated_pr_number), "reviewing", version .. "-" .. tostring(delegated_issue_number)),
          author_login = "fkst-test-bot",
          created_at = "2026-06-03T01:00:02Z",
        },
      },
      times = 2,
    })

    local scanned = run_liveness_scan(run_opts)
    t.eq(scanned.exit_code, 0, tostring(scanned.stderr or ""))
    local raised_by_number = {}
    for _, raised in ipairs(scanned.raises or {}) do
      if raised.queue == "devloop_observe_issue" and raised.payload ~= nil then
        raised_by_number[tonumber(raised.payload.number)] = raised
      end
    end
    t.eq(count_comment_stream_reads(), #numbers + 1)

    for _, number in ipairs(numbers) do
      local raised = raised_by_number[number]
      t.is_true(raised ~= nil)
      t.eq(raised.payload.source, "liveness-scan")
      local observed = run_observe_issue(raised.payload, run_opts)
      t.eq(observed.exit_code, 0, tostring(observed.stderr or ""))
    end

    t.eq(count_comment_stream_reads(), #numbers + 1)
  end,

  test_liveness_scan_reinjected_observe_reuses_same_validator_comment_stream = function()
    local run_opts = h.opts("entity-view-cycle-coalescing")
    mock_repo_env()
    mock_issue_list()
    mock_issue_state()
    codex_status.seed_role_codex_run(run_opts, "consensus", proposal_id, version)

    local scanned = run_liveness_scan(run_opts)
    t.eq(scanned.exit_code, 0, tostring(scanned.stderr or ""))
    local raised = h.find_raise(scanned.raises, "devloop_observe_issue", function(payload)
      return payload.source == "liveness-scan" and payload.updated_at == updated_at
    end)
    t.is_true(raised ~= nil)
    t.eq(raised.payload.source, "liveness-scan")
    t.eq(raised.payload.updated_at, updated_at)
    t.eq(count_comment_stream_reads(issue_number), 1)

    local observed = run_observe_issue(raised.payload, run_opts)
    t.eq(observed.exit_code, 0, tostring(observed.stderr or ""))
    t.eq(count_comment_stream_reads(issue_number), 1)
  end,
}
