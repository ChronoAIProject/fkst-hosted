local h = require("tests.devloop_helpers")
local fresh_issue_view_helpers = require("tests.fresh_issue_view_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local issue = h.issue
local run_open_pr = h.run_open_pr
local mock_issue_open_pr = h.mock_issue_open_pr
local count_calls = h.count_calls
local render_comment = h.render_comment
local run_observe = h.run_observe

local function full_issue_view(labels, comments, extra)
  local rendered_labels = {}
  for _, label in ipairs(labels or {}) do
    table.insert(rendered_labels, string.format('{"name":"%s"}', h.json_string(label)))
  end
  local rendered_comments = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered_comments, render_comment(comment))
  end
  local fields = extra or {}
  t.mock_command("--json title,body,comments,labels,state", {
    stdout = string.format(
      '{"title":"%s","body":"%s","state":"%s","labels":[%s],"comments":[%s]}\n',
      h.json_string(fields.title or "Implement decision recorder"),
      h.json_string(fields.body or ""),
      h.json_string(fields.state or "OPEN"),
      table.concat(rendered_labels, ","),
      table.concat(rendered_comments, ",")
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function shared_opts(name)
  return opts("entity-view-cache-" .. name)
end

local function assert_clean_open_pr_skip(result)
  t.eq(result.exit_code, 0)
  t.eq(#result.raises, 0)
  t.eq(count_calls("show-ref --verify --quiet"), 0)
  t.eq(count_calls("rev-parse --verify"), 0)
  t.eq(count_calls("git -C"), 0)
end

return {
  test_open_pr_skips_entity_changed_with_no_state_marker = function()
    mock_issue_open_pr({ "fkst-dev:enabled" }, {})

    local result = run_open_pr(issue({ labels = { "fkst-dev:enabled" } }), opts("open-pr-no-state-marker"))

    assert_clean_open_pr_skip(result)
  end,

  test_open_pr_skips_entity_changed_for_thinking_issue = function()
    mock_issue_open_pr({ "fkst-dev:thinking" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "thinking", "2026-06-02T00-00-00Z"),
    })

    local result = run_open_pr(issue({ labels = { "fkst-dev:thinking" } }), opts("open-pr-thinking"))

    assert_clean_open_pr_skip(result)
  end,

  test_open_pr_skips_entity_changed_for_ready_issue = function()
    mock_issue_open_pr({ "fkst-dev:ready" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "ready", "2026-06-02T00-00-00Z"),
    })

    local result = run_open_pr(issue({ labels = { "fkst-dev:ready" } }), opts("open-pr-ready"))

    assert_clean_open_pr_skip(result)
  end,

  test_open_pr_retries_when_implementing_fact_marker_missing = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_issue_open_pr({ "fkst-dev:implementing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "implementing", impl_version),
    })

    local result = run_open_pr(issue({ labels = { "fkst-dev:implementing" } }), opts("open-pr-missing-implementing-fact"))

    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("show-ref --verify --quiet"), 0)
    t.eq(count_calls("rev-parse --verify"), 0)
  end,

  test_issue_entity_view_is_shared_across_event_driven_departments = function()
    full_issue_view({ "fkst-dev:ready" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "ready", "ready/version"),
    })
    local run_opts = shared_opts("same-updated-at")
    local event = issue({ labels = { "fkst-dev:ready" }, updated_at = "2026-06-03T01:02:03Z" })

    local observed = run_observe(event, run_opts)
    local opened = run_open_pr(event, run_opts)

    t.eq(observed.exit_code, 0)
    t.eq(opened.exit_code, 0)
    t.eq(count_calls("gh issue view"), 1)
  end,

  test_issue_entity_view_cache_misses_on_different_updated_at = function()
    local run_opts = shared_opts("different-updated-at")
    full_issue_view({ "fkst-dev:ready" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "ready", "ready/version"),
    })
    local first = run_observe(issue({ labels = { "fkst-dev:ready" }, updated_at = "2026-06-03T01:02:03Z" }), run_opts)
    full_issue_view({ "fkst-dev:ready" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "ready", "ready/version"),
    })
    local second = run_observe(issue({ labels = { "fkst-dev:ready" }, updated_at = "2026-06-03T01:02:04Z" }), run_opts)

    t.eq(first.exit_code, 0)
    t.eq(second.exit_code, 0)
    t.eq(count_calls("gh issue view"), 2)
  end,

  test_issue_entity_view_fresh_bypasses_warm_cache = function()
    local run_opts = shared_opts("fresh-bypass")
    full_issue_view({ "fkst-dev:ready" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "ready", "ready/version"),
    })
    local cached = run_observe(issue({ labels = { "fkst-dev:ready" } }), run_opts)
    t.eq(cached.exit_code, 0)

    full_issue_view({ "fkst-dev:ready" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "ready", "ready/version"),
    })
    local fresh = t.run_department("tests/fresh_issue_view_helpers.lua", {
      queue = "test_fresh_issue_view",
      payload = {
        repo = "owner/repo",
        number = 42,
        updated_at = "2026-06-03T01:02:03Z",
      },
    }, run_opts)
    t.eq(fresh.exit_code, 0)
    t.eq(count_calls("gh issue view"), 2)
  end,
}
