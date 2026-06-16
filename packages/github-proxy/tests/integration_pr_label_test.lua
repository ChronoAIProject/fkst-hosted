local h = require("tests.proxy_integration_helpers")
local t = h.t
local opts = h.opts
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local mock_pr_label_guard = h.mock_pr_label_guard
local mock_repo_label_list = h.mock_repo_label_list
local calls_matching = h.calls_matching
local count_calls = h.count_calls

local function label_event(extra)
  local payload = {
    schema = "github-proxy.label.v1",
    repo = "owner/x",
    target_kind = "pr",
    target_number = 7,
    pr_number = 7,
    issue_number = 42,
    expected_proposal_id = "github-devloop/issue/owner/x/42",
    expected_state = "reviewing",
    expected_version = "v1",
    add_labels = { "fkst-dev:reviewing" },
    remove_labels = { "fkst-dev:pr-open" },
    dedup_key = "github-devloop/issue/owner/x/42/pr-label/reviewing/v1/7",
    source_ref = {
      kind = "external",
      ref = "owner/x#pr/7",
    },
    claim = {
      owner = "fkst-test-bot",
      source_ref = {
        kind = "external",
        ref = "owner/x#issue/42",
      },
    },
  }
  for key, value in pairs(extra or {}) do
    payload[key] = value
  end
  return {
    queue = "github_issue_label_request",
    payload = payload,
  }
end

local function state_marker(state, version)
  return '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="'
    .. state
    .. '" version="'
    .. version
    .. '" stage_rank="675" -->'
end

local function has_arg_pair(rendered, flag, value)
  local text = tostring(rendered or "")
  return text:find(tostring(flag) .. " '" .. tostring(value) .. "'", 1, true) ~= nil
    or text:find(tostring(flag) .. " " .. tostring(value), 1, true) ~= nil
end

return {
  test_pr_label_request_is_guarded_by_pr_comment_stream = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_label_guard({ "fkst-dev:pr-open" }, { state_marker("reviewing", "v1") })
    mock_repo_label_list({ "fkst-dev:reviewing", "fkst-dev:pr-open" })
    t.mock_command("gh pr edit", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("gh api repos/owner/x/issues/42", {
      stdout = '{"assignees":[{"login":"fkst-test-bot"}]}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = t.run_department("departments/github_issue_label/main.lua", label_event(), opts("pr-label-write", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api repos/owner/x/pulls/7"), 1)
    t.eq(count_calls("gh api --paginate --slurp repos/owner/x/issues/7/comments?per_page=100"), 1)
    t.eq(count_calls("gh api repos/owner/x/issues/42"), 1)
    t.eq(count_calls("gh pr edit"), 1)
    t.eq(count_calls("gh issue edit"), 0)
    local edit = calls_matching("gh pr edit")[1]
    t.is_true(has_arg_pair(edit.rendered, "--add-label", "fkst-dev:reviewing"))
    t.is_true(has_arg_pair(edit.rendered, "--remove-label", "fkst-dev:pr-open"))
  end,

  test_pr_label_request_retries_when_pr_marker_is_not_visible = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_label_guard({ "fkst-dev:pr-open" }, {})

    local result = t.run_department("departments/github_issue_label/main.lua", label_event(), opts("pr-label-marker-not-visible", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 1)
    t.eq(count_calls("gh api repos/owner/x/pulls/7"), 1)
    t.eq(count_calls("gh api --paginate --slurp repos/owner/x/issues/7/comments?per_page=100"), 1)
    t.eq(count_calls("gh pr edit"), 0)
    t.eq(count_calls("gh api repos/owner/x/issues/42"), 0)
  end,

  test_pr_label_request_skips_stale_visible_pr_marker = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_label_guard({ "fkst-dev:pr-open" }, { state_marker("merge-ready", "v1") })

    local result = t.run_department("departments/github_issue_label/main.lua", label_event(), opts("pr-label-stale-marker", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api repos/owner/x/pulls/7"), 1)
    t.eq(count_calls("gh api --paginate --slurp repos/owner/x/issues/7/comments?per_page=100"), 1)
    t.eq(count_calls("gh pr edit"), 0)
    t.eq(count_calls("gh api repos/owner/x/issues/42"), 0)
  end,
}
