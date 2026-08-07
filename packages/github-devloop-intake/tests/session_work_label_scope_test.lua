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
    queue = "github-proxy.github_issue_changed",
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
  for _ = 1, claim_mode_reads or 16 do
    t.mock_command('printf %s "$FKST_GITHUB_CLAIM_MODE"', { stdout = "label", stderr = "", exit_code = 0 })
  end
  for _ = 1, 16 do
    t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL"', { stdout = value or "", stderr = "", exit_code = 0 })
    t.mock_command(config.read_env_command("FKST_SESSION_WORK_LABEL_MAP_JSON"), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(config.read_env_command("FKST_WORK_LABEL_NAMESPACE"), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  end
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

local function assert_label_mode_rejects(number, name, labels, scope)
  mock_repo_env()
  mock_scope(scope == nil and "fkst-dev,fkst-security,fkst-workflow" or scope)
  mock_issue(number, labels)

  local result = run_admission(number, "session-work-label-reject-" .. name)

  t.eq(result.exit_code, 0)
  t.eq(candidate(result), nil)
  t.eq(h.count_calls("--add-label"), 0)
end

return {
  -- A scheduled-workflow RUN issue belongs to workflow-runner, never to the dev
  -- loop. It MUST carry a label in the session's effective set so the clock can
  -- wake the session with it, so the session-scope check admits it -- and the
  -- dev loop then works a run as a feature. Observed in production (#5904): run
  -- issue #5896 was claimed and merged as a PR re-implementing the workflow
  -- definition, while the run itself never executed.
  --
  -- The scope value here deliberately INCLUDES the run label, reproducing the
  -- real session: it is in the set, and the issue must still be declined.
  test_a_scheduled_workflow_run_issue_is_never_admitted_by_the_dev_loop = function()
    for _, label in ipairs({
      "fkst-workflow-run",
      "fkst-workflow-run-chronoai-fkst-cloud",
      "fkst-workflow-scheduled",
      "fkst-workflow-scheduled-chronoai-fkst-cloud",
    }) do
      mock_repo_env()
      mock_scope("fkst-dev,fkst-security,fkst-workflow," .. label)
      mock_issue(77, { label })

      local result = run_admission(77, "workflow-run-declined-" .. label)

      t.eq(result.exit_code, 0)
      t.eq(candidate(result), nil)
      t.eq(h.count_calls("--add-label"), 0)
    end
  end,

  -- The prefix match must not swallow the AUTHORING queue. `fkst-workflow` is
  -- workflow-writer's label and is ordinary work; only the run family belongs to
  -- the runner.
  test_the_workflow_authoring_label_is_not_mistaken_for_a_run = function()
    mock_repo_env()
    mock_scope("fkst-dev,fkst-security,fkst-workflow")
    mock_issue(78, { "fkst-workflow" })

    local result = run_admission(78, "workflow-authoring-still-admitted")

    t.eq(result.exit_code, 0)
    t.is_true(candidate(result) ~= nil, "an authoring issue must still be admitted")
  end,

  test_session_work_label_parser_trims_and_deduplicates_exact_labels = function()
    local labels = config.parse_session_work_labels(" fkst-dev, fkst-security, fkst-dev ,,fkst-workflow ")
    t.eq(#labels, 3)
    t.eq(labels[1], "fkst-dev")
    t.eq(labels[2], "fkst-security")
    t.eq(labels[3], "fkst-workflow")
  end,

  test_materialized_child_fkst_dev_label_passes_exact_session_scope = function()
    mock_repo_env()
    mock_scope(" fkst-dev, fkst-security, fkst-dev, fkst-workflow ", 3)
    mock_issue(42, { "fkst-dev" })

    local result = run_admission(42, "materialized-child-work-label-exact")

    t.eq(result.exit_code, 0)
    t.is_true(candidate(result) ~= nil)
    t.eq(candidate(result).payload.issue_number, "42")
    t.eq(h.count_calls("--add-label"), 0)
  end,

  test_namespaced_cloud_label_passes_exact_session_scope = function()
    mock_repo_env()
    mock_scope("fkst-dev-chronoai-fkst", 3)
    mock_issue(49, { "fkst-dev-chronoai-fkst" })

    local result = run_admission(49, "namespaced-session-work-label-exact")

    t.eq(result.exit_code, 0)
    t.is_true(candidate(result) ~= nil)
    t.eq(candidate(result).payload.issue_number, "49")
  end,

  test_plain_label_is_rejected_by_namespaced_cloud_session = function()
    mock_repo_env()
    mock_scope("fkst-dev-chronoai-fkst")
    mock_issue(50, { "fkst-dev" })

    local result = run_admission(50, "namespaced-session-rejects-plain-label")

    t.eq(result.exit_code, 0)
    t.eq(candidate(result), nil)
    t.eq(h.count_calls("--add-label"), 0)
  end,

  test_plain_assignee_session_rejects_namespaced_and_dual_work_labels_before_claim = function()
    local cloud = "fkst-dev-chronoai-fkst-cloud-test"
    for index, labels in ipairs({
      { cloud },
      { "fkst-dev", cloud },
    }) do
      mock_repo_env()
      for _ = 1, 4 do
        t.mock_command('printf %s "$FKST_GITHUB_CLAIM_MODE"', {
          stdout = "assignee",
          stderr = "",
          exit_code = 0,
        })
        t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL"', {
          stdout = "fkst-dev",
          stderr = "",
          exit_code = 0,
        })
        t.mock_command(config.read_env_command("FKST_SESSION_WORK_LABEL_MAP_JSON"), {
          stdout = "",
          stderr = "",
          exit_code = 0,
        })
        t.mock_command(config.read_env_command("FKST_WORK_LABEL_NAMESPACE"), {
          stdout = "",
          stderr = "",
          exit_code = 0,
        })
      end
      mock_issue(50 + index, labels)

      local result = t.run_department("departments/admission/main.lua", event(50 + index), h.opts(
        "plain-assignee-session-rejects-foreign-" .. tostring(index),
        {
          FKST_GITHUB_CLAIM_MODE = "assignee",
          FKST_GITHUB_REPO = "owner/repo",
          FKST_GITHUB_WRITE = "1",
          FKST_SESSION_WORK_LABEL = "fkst-dev",
        }
      ))

      t.eq(result.exit_code, 0)
      t.eq(candidate(result), nil)
      t.eq(h.count_calls("--add-assignee"), 0)
      t.eq(h.count_calls("--add-label"), 0)
    end
  end,

  test_label_mode_rejects_lifecycle_suffix_without_base_work_label = function()
    assert_label_mode_rejects(43, "prefix-only", { "fkst-dev:thinking" })
  end,

  test_label_mode_rejects_session_trigger_label = function()
    assert_label_mode_rejects(44, "trigger", { "fkst-session" })
  end,

  test_label_mode_rejects_dashboard_label = function()
    assert_label_mode_rejects(45, "dashboard", { "fkst-dashboard" })
  end,

  test_label_mode_rejects_unrelated_label = function()
    assert_label_mode_rejects(46, "unrelated", { "bug" })
  end,

  test_label_mode_rejects_when_session_work_scope_is_empty = function()
    assert_label_mode_rejects(47, "blank-scope", { "fkst-dev" }, "")
  end,

  test_pull_request_is_rejected_without_issue_read_or_claim = function()
    local result = run_admission(48, "session-work-label-reject-pr", "pr")

    t.eq(result.exit_code, 0)
    t.eq(candidate(result), nil)
    t.eq(h.count_calls("gh issue view"), 0)
    t.eq(h.count_calls("--add-label"), 0)
  end,
}
