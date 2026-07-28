local h = require("tests.proxy_integration_helpers")
local t = h.t

local issue = h.poll_issue_json(60, "2026-06-03T01:02:00Z")

local function issue_with_labels(number, labels)
  local rendered = {}
  for _, label in ipairs(labels) do
    rendered[#rendered + 1] = string.format('{"name":"%s"}', label)
  end
  return string.format(
    '{"number":%d,"title":"Issue %d","html_url":"https://github.example/owner/x/issues/%d","updated_at":"2026-06-03T01:%02d:00Z","state":"open","author":{"login":"fkst-test-bot"},"labels":[%s],"assignees":[]}',
    number,
    number,
    number,
    number,
    table.concat(rendered, ",")
  )
end

local function mock_poll()
  h.mock_repo_env()
  h.mock_poll_label_prefix_env("adapter-")
  h.mock_proxy_replay_budget_env("10")
  h.mock_issue_list(h.poll_issue_list_from({ issue }))
  h.mock_pr_list("[]\n")
end

local function run_poll(run_opts, token)
  return t.run_department("departments/github_poll/main.lua", {
    queue = "github_poll_tick",
    payload = {},
    ts = token,
  }, run_opts)
end

local function mock_plain_session_scope(label, reads)
  for _ = 1, reads or 4 do
    t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL"', {
      stdout = label,
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL_MAP_JSON"', {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_WORK_LABEL_NAMESPACE"', {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  end
end

return {
  test_assignee_mode_emits_observed_issue_replay = function()
    local run_opts = h.opts("assignee-mode-observation", {
      FKST_GITHUB_CLAIM_MODE = "assignee",
    })

    mock_poll()
    local first = run_poll(run_opts, "assignee-poll-1")
    t.eq(first.exit_code, 0)
    t.eq(#h.changed_raises(first.raises), 1)

    mock_poll()
    local second = run_poll(run_opts, "assignee-poll-2")
    t.eq(second.exit_code, 0)
    t.eq(#h.changed_raises(second.raises), 0)
    t.eq(#h.observed_issue_raises(second.raises), 1)
  end,

  test_label_mode_suppresses_observed_issue_replay = function()
    local run_opts = h.opts("label-mode-observation", {
      FKST_GITHUB_CLAIM_MODE = "label",
    })

    mock_poll()
    local first = run_poll(run_opts, "label-poll-1")
    t.eq(first.exit_code, 0)
    t.eq(#h.changed_raises(first.raises), 1)

    mock_poll()
    local second = run_poll(run_opts, "label-poll-2")
    t.eq(second.exit_code, 0)
    t.eq(#h.changed_raises(second.raises), 0)
    t.eq(#h.observed_issue_raises(second.raises), 0)
    t.eq(#second.raises, 0)
  end,

  test_plain_assignee_poll_rejects_cloud_and_ambiguous_dual_label_issues = function()
    local logical = "fkst-dev"
    local cloud = "fkst-dev-chronoai-fkst-cloud-test"
    local run_opts = h.opts("plain-assignee-poll-scope", {
      FKST_GITHUB_CLAIM_MODE = "assignee",
      FKST_SESSION_WORK_LABEL = logical,
    })
    mock_plain_session_scope(logical)
    h.mock_repo_env()
    h.mock_poll_label_prefix_env(logical .. ":")
    h.mock_proxy_replay_budget_env("10")
    h.mock_issue_list(h.poll_issue_list_from({
      issue_with_labels(10, { logical }),
      issue_with_labels(11, { cloud }),
      issue_with_labels(12, { logical, cloud }),
      issue_with_labels(13, { logical, logical .. ":thinking" }),
      issue_with_labels(14, { cloud, cloud .. ":thinking" }),
    }))
    h.mock_pr_list("[]\n")

    local result = run_poll(run_opts, "plain-assignee-poll")
    t.eq(result.exit_code, 0)
    local changed = h.changed_raises(result.raises)
    t.eq(#changed, 2)
    t.eq(changed[1].payload.number, 10)
    t.eq(changed[2].payload.number, 13)
  end,

  test_namespaced_poll_emits_only_issues_with_the_exact_effective_base_label = function()
    local logical = "fkst-dev"
    local namespace = "chronoai-fkst-cloud-test"
    local effective = "fkst-dev-chronoai-fkst-cloud-test"
    local run_opts = h.opts("namespaced-poll-scope", {
      FKST_GITHUB_CLAIM_MODE = "label",
      FKST_SESSION_WORK_LABEL = effective,
      FKST_SESSION_WORK_LABEL_MAP_JSON = string.format('{"%s":"%s"}', logical, effective),
      FKST_WORK_LABEL_NAMESPACE = namespace,
    })
    for _ = 1, 2 do
      t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL"', {
        stdout = effective,
        stderr = "",
        exit_code = 0,
      })
    end
    t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL_MAP_JSON"', {
      stdout = string.format('{"%s":"%s"}', logical, effective),
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_WORK_LABEL_NAMESPACE"', {
      stdout = namespace,
      stderr = "",
      exit_code = 0,
    })
    h.mock_repo_env()
    h.mock_poll_label_prefix_env(effective .. ":")
    h.mock_proxy_replay_budget_env("10")
    h.mock_issue_list(h.poll_issue_list_from({
      issue_with_labels(1, { effective }),
      issue_with_labels(2, { "fkst-dev" }),
      issue_with_labels(3, { "fkst-dev:claimed" }),
      issue_with_labels(4, { effective .. ":claimed" }),
      issue_with_labels(5, { "fkst-dev-another-provider" }),
      issue_with_labels(6, { effective, effective .. ":thinking" }),
      issue_with_labels(8, { effective, "fkst-dev" }),
    }))
    h.mock_pr_list("[]\n")

    local result = run_poll(run_opts, "namespaced-poll")
    t.eq(result.exit_code, 0)
    local changed = h.changed_raises(result.raises)
    t.eq(#changed, 2)
    t.eq(changed[1].payload.number, 1)
    t.eq(changed[2].payload.number, 6)
  end,

  test_namespaced_poll_fails_closed_when_session_work_labels_are_empty = function()
    local logical = "fkst-dev"
    local namespace = "chronoai-fkst-cloud-test"
    local effective = "fkst-dev-chronoai-fkst-cloud-test"
    local map_json = string.format('{"%s":"%s"}', logical, effective)
    local run_opts = h.opts("namespaced-poll-empty-scope", {
      FKST_GITHUB_CLAIM_MODE = "label",
      FKST_SESSION_WORK_LABEL = "",
      FKST_SESSION_WORK_LABEL_MAP_JSON = map_json,
      FKST_WORK_LABEL_NAMESPACE = namespace,
    })
    for _ = 1, 2 do
      t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL"', {
        stdout = "",
        stderr = "",
        exit_code = 0,
      })
    end
    for _ = 1, 2 do
      t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL_MAP_JSON"', {
        stdout = map_json,
        stderr = "",
        exit_code = 0,
      })
    end
    t.mock_command('printf %s "$FKST_WORK_LABEL_NAMESPACE"', {
      stdout = namespace,
      stderr = "",
      exit_code = 0,
    })
    h.mock_repo_env()
    h.mock_poll_label_prefix_env(effective .. ":")
    h.mock_proxy_replay_budget_env("10")
    h.mock_issue_list(h.poll_issue_list_from({
      issue_with_labels(7, { effective }),
    }))
    h.mock_pr_list("[]\n")

    local result = run_poll(run_opts, "namespaced-poll-empty-scope")
    t.eq(result.exit_code, 0)
    t.eq(#h.changed_raises(result.raises), 0)
    t.eq(#result.raises, 0)
  end,
}
