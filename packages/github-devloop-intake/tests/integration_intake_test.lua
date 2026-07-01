local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")

local function mock_repo_env(repo)
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_ROLLUP_MERGE"', { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = repo or "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "", stderr = "", exit_code = 0 })
end

local function source_ref(number)
  return entity_lib.issue_source_ref("owner/repo", number or 42)
end

local function entity_changed(number, fields)
  local f = fields or {}
  local selected = number or f.number or 42
  local updated_at = f.updated_at or "2026-06-03T01:02:03Z"
  return {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = f.type or "issue",
      repo = f.repo or "owner/repo",
      number = selected,
      title = f.title or "Issue",
      state = f.state or "OPEN",
      labels = f.labels or {},
      updated_at = updated_at,
      dedup_key = tostring(f.repo or "owner/repo") .. "#issue#" .. tostring(selected) .. "@" .. tostring(updated_at),
      source_ref = source_ref(selected),
    },
    source_ref = source_ref(selected),
  }
end

local function mock_issue(number, fields)
  local f = fields or {}
  entity_read_mocks.mock_issue_view_selector(t, {
    number = number or 42,
    title = f.title or "Issue",
    body = f.body or "",
    updated_at = f.updated_at or "2026-06-03T01:02:03Z",
    state = f.state or "OPEN",
    labels = f.labels or {},
    comments = f.comments or {},
    assignees = f.assignees or { "fkst-test-bot" },
    author_login = f.author_login or "fkst-test-bot",
  }, "title,body,createdAt,updatedAt,labels,comments,state,assignees,author")
end

local function run_admission(event, run_opts)
  return t.run_department("departments/admission/main.lua", event or entity_changed(42), run_opts)
end

local function trusted_reintake_command(id)
  return {
    id = id or "IC_reintake_1",
    body = "fkst: reintake",
    author_login = devloop_base.trusted_bot_login(),
    created_at = "2026-06-04T03:00:00Z",
  }
end

local function untrusted_reintake_command(id)
  local command = trusted_reintake_command(id or "IC_reintake_untrusted")
  command.author_login = "ordinary-user"
  return command
end

local function find_comment_body(raises, needle)
  for _, raised in ipairs(raises or {}) do
    if raised.queue == "github-proxy.github_issue_comment_request"
      and raised.payload.body:find(needle, 1, true) ~= nil then
      return raised.payload
    end
  end
  return nil
end

local function expected_effect_key(proposal_id, command)
  return devloop_base.intake_decision_dedup_key(proposal_id, { title = "Issue", body = "" }, command)
end

return {
  test_admission_filters_non_issue_closed_known_hold_and_trusted_marker = function()
    local cases = {
      { name = "pr", event = entity_changed(42, { type = "pr" }), view = nil },
      { name = "closed-event", event = entity_changed(42, { state = "CLOSED" }), view = nil },
      { name = "enabled", event = entity_changed(40), view = { number = 40, labels = { "fkst-dev:enabled" } } },
      { name = "thinking", event = entity_changed(41), view = { number = 41, labels = { "fkst-dev:thinking" } } },
      { name = "hold", event = entity_changed(42), view = { number = 42, labels = { "fkst-dev:hold" } } },
      {
        name = "trusted-marker",
        event = entity_changed(45),
        view = {
          number = 45,
          comments = {
            m_builders.intake_decision_marker(core, "github-devloop/issue/owner/repo/45", "decline", "intake/github-devloop/issue/owner/repo/45/v1", "standard"),
          },
        },
      },
    }
    for _, case in ipairs(cases) do
      h.mock_bot_env()
      mock_repo_env()
      if case.view ~= nil then
        mock_issue(case.view.number, case.view)
      end
      local result = run_admission(case.event, opts("intake-admission-filter-" .. case.name))
      t.eq(result.exit_code, 0)
      t.eq(#result.raises, 0)
    end
  end,

  test_admission_raises_candidate_for_open_unmanaged_issue = function()
    h.mock_bot_env()
    mock_repo_env()
    mock_issue(43, { labels = { "fkst-class:expedite" } })

    local result = run_admission(entity_changed(43), opts("intake-admission-open"))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "devloop_intake_candidate")
    t.eq(result.raises[1].payload.issue_number, "43")
    t.eq(result.raises[1].payload.source_ref.ref, "owner/repo#issue/43")
  end,

  test_admission_reintake_requeues_issue_with_trusted_intake_marker = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local command = trusted_reintake_command("IC_reintake_admission")
    h.mock_bot_env()
    mock_repo_env()
    mock_issue(42, {
      comments = {
        m_builders.intake_decision_marker(core, proposal_id, "escalate-to-class", "intake/github-devloop/issue/owner/repo/42/v1", "standard"),
        command,
      },
    })

    local result = run_admission(entity_changed(42), opts("intake-admission-reintake"))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "devloop_intake_candidate")
    t.eq(result.raises[1].payload.issue_number, "42")
    t.eq(result.raises[1].payload.effect_id, expected_effect_key(proposal_id, command))
    t.eq(result.raises[1].payload.reintake_command_created_at, command.created_at)
    t.is_true(result.raises[1].payload.dedup_key:find("intake%-candidate/github%-devloop/issue/owner/repo/42", 1, false) ~= nil)
  end,

  test_admission_reintake_without_prior_intake_marker_refuses = function()
    h.mock_bot_env()
    mock_repo_env()
    mock_issue(42, { comments = { trusted_reintake_command("IC_reintake_no_marker") } })

    local result = run_admission(entity_changed(42), opts("intake-admission-reintake-no-marker"))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local refusal = find_comment_body(result.raises, "operator command refused")
    t.is_true(refusal ~= nil)
    t.is_true(refusal.body:find("reintake requires an existing intake decision", 1, true) ~= nil)
    t.is_true(refusal.body:find('outcome="refused"', 1, true) ~= nil)
  end,

  test_admission_reintake_mid_pipeline_refuses = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    h.mock_bot_env()
    mock_repo_env()
    mock_issue(42, {
      labels = { "fkst-dev:thinking" },
      comments = {
        m_builders.intake_decision_marker(core, proposal_id, "decline", "intake/github-devloop/issue/owner/repo/42/v1", "standard"),
        trusted_reintake_command("IC_reintake_active"),
      },
    })

    local result = run_admission(entity_changed(42), opts("intake-admission-reintake-active-state"))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local refusal = find_comment_body(result.raises, "operator command refused")
    t.is_true(refusal ~= nil)
    t.is_true(refusal.body:find("reintake requires no active devloop state", 1, true) ~= nil)
    t.is_true(refusal.body:find('outcome="refused"', 1, true) ~= nil)
  end,

  test_admission_reintake_forged_command_is_ignored = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    h.mock_bot_env()
    mock_repo_env()
    mock_issue(42, {
      comments = {
        m_builders.intake_decision_marker(core, proposal_id, "decline", "intake/github-devloop/issue/owner/repo/42/v1", "standard"),
        untrusted_reintake_command("IC_reintake_forged"),
      },
    })

    local result = run_admission(entity_changed(42), opts("intake-admission-reintake-forged"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_admission_ignores_forged_marker = function()
    h.mock_bot_env()
    mock_repo_env()
    mock_issue(42, {
      comments = {
        {
          body = m_builders.intake_decision_marker(core, "github-devloop/issue/owner/repo/42", "decline", "intake/github-devloop/issue/owner/repo/42/v1", "standard"),
          author_login = "ordinary-user",
        },
      },
    })

    local result = run_admission(entity_changed(42), opts("intake-admission-forged-marker"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].payload.issue_number, "42")
  end,
}
