local config = require("devloop.config")
local entity_lib = require("devloop.entity")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local h = require("tests.devloop_helpers")
local t = h.t

local function source_ref(number)
  return entity_lib.issue_source_ref("owner/repo", number)
end

local function event(number, entity_type)
  return {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = entity_type or "issue",
      repo = "owner/repo",
      number = number,
      title = "Scoped work",
      state = "OPEN",
      labels = {},
      updated_at = "2026-07-21T01:02:03Z",
      dedup_key = "owner/repo#issue#" .. tostring(number) .. "@2026-07-21T01:02:03Z",
      source_ref = source_ref(number),
    },
    source_ref = source_ref(number),
  }
end

local function mock_repo_env()
  h.mock_bot_env()
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "1", stderr = "", exit_code = 0 })
end

local function mock_scope(value, claim_mode_reads)
  for _ = 1, claim_mode_reads or 1 do
    t.mock_command('printf %s "$FKST_GITHUB_CLAIM_MODE"', { stdout = "label", stderr = "", exit_code = 0 })
  end
  t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL"', { stdout = value or "", stderr = "", exit_code = 0 })
end

local function mock_issue(number, labels)
  entity_read_mocks.mock_issue_view_selector(t, {
    number = number,
    title = "Scoped work",
    body = "",
    updated_at = "2026-07-21T01:02:03Z",
    state = "OPEN",
    labels = labels,
    comments = {},
    assignees = {},
    author_login = "fkst-test-bot",
  }, "title,body,createdAt,updatedAt,labels,comments,state,assignees,author")
end

local function run_admission(number, name, entity_type)
  return t.run_department("departments/admission/main.lua", event(number, entity_type), h.opts(name, {
    FKST_GITHUB_CLAIM_MODE = "label",
    FKST_GITHUB_REPO = "owner/repo",
    FKST_GITHUB_WRITE = "1",
    FKST_SESSION_WORK_LABEL = "fkst-dev,fkst-security,fkst-workflow",
  }))
end

local function candidate(result)
  return h.find_raise(result.raises, "devloop_intake_candidate")
end

return {
  test_session_work_label_parser_trims_and_deduplicates_exact_labels = function()
    local labels = config.parse_session_work_labels(" fkst-dev, fkst-security, fkst-dev ,,fkst-workflow ")
    t.eq(#labels, 3)
    t.eq(labels[1], "fkst-dev")
    t.eq(labels[2], "fkst-security")
    t.eq(labels[3], "fkst-workflow")
  end,

  test_label_mode_admits_when_any_exact_session_work_label_matches = function()
    mock_repo_env()
    mock_scope(" fkst-dev, fkst-security, fkst-dev, fkst-workflow ", 3)
    mock_issue(42, { "bug", "fkst-security", "fkst-dev:claimed" })

    local result = run_admission(42, "session-work-label-exact")

    t.eq(result.exit_code, 0)
    t.is_true(candidate(result) ~= nil)
    t.eq(candidate(result).payload.issue_number, "42")
    t.eq(h.count_calls("--add-label"), 0)
  end,

  test_label_mode_rejects_non_work_entities_before_claim = function()
    local cases = {
      { number = 43, name = "prefix-only", labels = { "fkst-dev:thinking" } },
      { number = 44, name = "trigger", labels = { "fkst-session" } },
      { number = 45, name = "dashboard", labels = { "fkst-dashboard" } },
      { number = 46, name = "unrelated", labels = { "bug" } },
      { number = 47, name = "blank-scope", labels = { "fkst-dev" }, scope = "" },
    }

    for _, case in ipairs(cases) do
      mock_repo_env()
      mock_scope(case.scope == nil and "fkst-dev,fkst-security,fkst-workflow" or case.scope)
      mock_issue(case.number, case.labels)

      local result = run_admission(case.number, "session-work-label-reject-" .. case.name)

      t.eq(result.exit_code, 0)
      t.eq(candidate(result), nil)
      t.eq(h.count_calls("--add-label"), 0)
    end
  end,

  test_pull_request_is_rejected_without_issue_read_or_claim = function()
    local result = run_admission(48, "session-work-label-reject-pr", "pr")

    t.eq(result.exit_code, 0)
    t.eq(candidate(result), nil)
    t.eq(h.count_calls("gh issue view"), 0)
    t.eq(h.count_calls("--add-label"), 0)
  end,
}
