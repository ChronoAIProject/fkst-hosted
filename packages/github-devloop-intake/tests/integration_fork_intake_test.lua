local entity_lib = require("devloop.entity")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local find_raise = h.find_raise
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local function mock_repo_env()
  h.mock_bot_env()
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_ROLLUP_MERGE"', { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_FORK_GRACE_HOURS"', { stdout = "", stderr = "", exit_code = 0 })
end

local function source_ref()
  return entity_lib.issue_source_ref("owner/repo", 42)
end

local function event(updated_at)
  local selected_updated_at = updated_at or "2026-06-03T01:02:03Z"
  return {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "issue",
      repo = "owner/repo",
      number = 42,
      title = "External request",
      state = "OPEN",
      labels = {},
      updated_at = selected_updated_at,
      dedup_key = "owner/repo#issue#42@" .. selected_updated_at,
      source_ref = source_ref(),
    },
    source_ref = source_ref(),
  }
end

local function mock_admission_view(fields)
  local f = fields or {}
  entity_read_mocks.mock_issue_view_selector(t, {
    number = f.number or 42,
    title = "External request",
    body = "",
    created_at = f.created_at or "2026-06-03T01:00:00Z",
    updated_at = f.updated_at or "2026-06-03T01:02:03Z",
    state = f.state or "OPEN",
    labels = {},
    comments = {},
    assignees = {},
    author_login = "human",
  }, "title,body,createdAt,updatedAt,labels,comments,state,assignees,author")
end

local function mock_state_view(fields)
  local f = fields or {}
  t.mock_command(core.gh_issue_view_state_cmd("owner/repo", tostring(f.number or 42)), {
    stdout = '{"title":"External request","createdAt":"' .. tostring(f.created_at or "2026-06-03T01:00:00Z") .. '","updatedAt":"' .. tostring(f.updated_at or "2026-06-03T01:02:03Z") .. '","state":"' .. tostring(f.state or "OPEN") .. '","labels":[],"comments":[],"assignees":[],"author":{"login":"human"}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function run_admission(run_opts, updated_at)
  return t.run_department("departments/admission/main.lua", event(updated_at), run_opts)
end

local function assert_no_fork_or_candidate(result)
  t.eq(result.exit_code, 0)
  t.eq(#result.raises, 0)
  t.eq(find_raise(result.raises, "github-proxy.github_issue_create_request"), nil)
  t.eq(find_raise(result.raises, "devloop_intake_candidate"), nil)
end

local function created_inside_grace()
  return os.date("!%Y-%m-%dT%H:%M:%SZ", now())
end

local function created_after_grace()
  return os.date("!%Y-%m-%dT%H:%M:%SZ", now() - (3 * 60 * 60) - 1)
end

return {
  test_admission_other_authored_unassigned_issue_inside_grace_does_not_fork = function()
    mock_repo_env()
    mock_admission_view({ created_at = created_inside_grace() })
    mock_state_view({ created_at = created_inside_grace() })

    local result = run_admission(opts("fork-intake-admission-other-author"))

    assert_no_fork_or_candidate(result)
  end,

  test_admission_other_authored_unassigned_issue_after_grace_raises_fork_request_only = function()
    local run_opts = opts("fork-intake-admission-other-author-stale")
    mock_repo_env()
    mock_admission_view({ created_at = created_after_grace() })
    mock_state_view({ created_at = created_after_grace() })

    local result = run_admission(run_opts)

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local request = find_raise(result.raises, "github-proxy.github_issue_create_request").payload
    t.eq(request.external_effect_saga, "fork-and-block")
    t.eq(request.external_effect_step, "create-fork")
    t.eq(request.assignees[1], "fkst-test-bot")
    t.eq(request.parent_comment_target.issue_number, 42)
    t.eq(request.post_create_blocked_by.blocked_issue_number, 42)
    t.eq(request.post_create_blocked_by.external_effect_saga, "fork-and-block")
    t.eq(request.post_create_blocked_by.external_effect_step, "block-original")
    t.eq(find_raise(result.raises, "devloop_intake_candidate"), nil)
  end,

  test_admission_stale_open_issue_revalidates_closed_issue_before_fork = function()
    local run_opts = opts("fork-intake-admission-stale-open-author-closed")
    mock_repo_env()
    mock_admission_view({ created_at = created_after_grace() })
    mock_state_view({ state = "CLOSED", created_at = created_after_grace() })

    local result = run_admission(run_opts)

    assert_no_fork_or_candidate(result)
  end,

  test_admission_other_authored_closed_issue_after_grace_does_not_fork = function()
    local run_opts = opts("fork-intake-admission-other-author-closed")
    mock_repo_env()
    mock_admission_view({ state = "CLOSED", created_at = created_after_grace() })

    local result = run_admission(run_opts)

    assert_no_fork_or_candidate(result)
  end,

  test_admission_updated_at_change_does_not_restart_fork_grace = function()
    local run_opts = opts("fork-intake-admission-progress-keeps-grace")
    mock_repo_env()
    mock_admission_view({ created_at = created_after_grace(), updated_at = "2026-06-03T02:00:00Z" })
    mock_state_view({ created_at = created_after_grace(), updated_at = "2026-06-03T02:00:00Z" })

    local result = run_admission(run_opts, "2026-06-03T02:00:00Z")

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_create_request").payload.post_create_blocked_by.blocked_issue_number, 42)
    t.eq(find_raise(result.raises, "devloop_intake_candidate"), nil)
  end,
}
