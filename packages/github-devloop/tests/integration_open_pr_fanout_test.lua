local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local issue = h.issue
local reviewing = h.reviewing
local run_observe_pr = h.run_observe_pr
local source_ref = h.source_ref
local run_open_pr = h.run_open_pr
local mock_issue_open_pr = h.mock_issue_open_pr
local mock_branch_exists = h.mock_branch_exists
local mock_branch_head_descends = h.mock_branch_head_descends
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local count_calls = h.count_calls
local render_comment = h.render_comment
local run_observe = h.run_observe
local find_raise = h.find_raise
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local function full_issue_view(labels, comments, extra)
  local fields = extra or {}
  entity_read_mocks.mock_issue_view_selector(t, {
    title = fields.title or "Implement decision recorder",
    body = fields.body or "",
    state = fields.state or "OPEN",
    updated_at = fields.updated_at or "2026-06-03T01:02:03Z",
    labels = labels,
    comments = comments,
    assignees = { fields.assignee_login or "fkst-test-bot" },
    author_login = fields.author_login or "fkst-test-bot",
  }, "title,body,comments,labels,state,updatedAt,assignees")
end

local function issue_updated_at(value)
  t.mock_command("gh api 'repos/owner/repo/issues/42' --jq '.updated_at // .updatedAt // \"\"'", {
    stdout = tostring(value or "") .. "\n",
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
  test_open_pr_direct_kickoff_raises_pr_open_request = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local event = core.build_devloop_open_pr_payload("owner/repo", 42, {
      proposal_id = "github-devloop/issue/owner/repo/42",
      dedup_key = impl_version,
      source_ref = source_ref(),
    }, "devloop-owner-repo-42-01HY", "abc123", "dev")
    mock_issue_open_pr({ "fkst-dev:implementing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "implementing", impl_version),
      core.implementing_marker("github-devloop/issue/owner/repo/42", impl_version, "devloop-owner-repo-42-01HY", "abc123", "dev", "abc123"),
    })
    mock_branch_exists("devloop-owner-repo-42-01HY", "abc123")
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")

    local result = run_open_pr(event, opts("open-pr-direct-write", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local pr_raise = find_raise(result.raises, "github-proxy.github_pr_open_request")
    t.eq(pr_raise.payload.schema, "github-proxy.pr-open.v1")
    t.eq(pr_raise.payload.branch, "devloop-owner-repo-42-01HY")
    t.eq(pr_raise.payload.head_sha, "abc123")
    t.eq(pr_raise.payload.impl_version, impl_version)
  end,

  test_open_pr_entity_change_opens_at_current_descendant_head = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_issue_open_pr({ "fkst-dev:implementing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "implementing", impl_version),
      core.implementing_marker("github-devloop/issue/owner/repo/42", impl_version, "devloop-owner-repo-42-01HY", "abc123", "dev", "abc123"),
    })
    mock_branch_exists("devloop-owner-repo-42-01HY", "def456")
    mock_branch_head_descends(true)
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")

    local result = run_open_pr(issue({ labels = { "fkst-dev:implementing" } }), opts("open-pr-descendant-head", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local pr_raise = find_raise(result.raises, "github-proxy.github_pr_open_request")
    t.eq(pr_raise.payload.branch, "devloop-owner-repo-42-01HY")
    t.eq(pr_raise.payload.head_sha, "def456")
    t.eq(pr_raise.payload.impl_version, impl_version)
    t.eq(count_calls("show-ref --verify --quiet"), 1)
    t.eq(count_calls("rev-parse --verify"), 1)
    t.eq(count_calls("merge-base --is-ancestor"), 1)
  end,

  test_open_pr_entity_change_refuses_non_descendant_head = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_issue_open_pr({ "fkst-dev:implementing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "implementing", impl_version),
      core.implementing_marker("github-devloop/issue/owner/repo/42", impl_version, "devloop-owner-repo-42-01HY", "abc123", "dev", "abc123"),
    })
    mock_branch_exists("devloop-owner-repo-42-01HY", "def456")
    mock_branch_head_descends(false)
    mock_bot_env()
    mock_write_env("1")

    local result = run_open_pr(issue({ labels = { "fkst-dev:implementing" } }), opts("open-pr-non-descendant-head", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("show-ref --verify --quiet"), 1)
    t.eq(count_calls("rev-parse --verify"), 1)
    t.eq(count_calls("merge-base --is-ancestor"), 1)
  end,

  test_open_pr_redrive_repairs_stale_blocked_state_label = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/1"
    mock_issue_open_pr({ "fkst-dev:blocked" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "pr-open", impl_version),
    })
    mock_bot_env()

    local result = run_open_pr(issue({
      labels = { "fkst-dev:blocked" },
      source_ref = source_ref(),
    }), opts("open-pr-redrive-stale-blocked-label"))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:pr-open")
    t.is_true(h.has_value(label_raise.payload.remove_labels, "fkst-dev:blocked"))
    t.is_true(h.has_value(label_raise.payload.remove_labels, "fkst-dev:impl-failed"))
    t.is_true(h.has_value(label_raise.payload.remove_labels, "fkst-dev:merged"))
    t.eq(h.has_value(label_raise.payload.remove_labels, "fkst-dev:pr-open"), false)
    t.eq(count_calls("show-ref --verify --quiet"), 0)
    t.eq(count_calls("rev-parse --verify"), 0)
  end,

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

  test_marker_bearing_issue_view_is_fresh_across_event_driven_departments = function()
    full_issue_view({ "fkst-dev:ready" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "ready", "ready/version"),
    })
    local run_opts = shared_opts("same-updated-at")
    local event = issue({ labels = { "fkst-dev:ready" }, updated_at = "2026-06-03T01:02:03Z" })

    local observed = run_observe(event, run_opts)
    full_issue_view({ "fkst-dev:ready" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "ready", "ready/version"),
    })
    local opened = run_open_pr(event, run_opts)

    t.eq(observed.exit_code, 0)
    t.eq(opened.exit_code, 0)
  end,

  test_cross_consumer_delayed_retry_refetches_current_issue_truth = function()
    local run_opts = shared_opts("cross-consumer-delayed-retry")
    local event = issue({ labels = { "fkst-dev:ready" }, updated_at = "2026-06-03T01:02:03Z" })
    full_issue_view({ "fkst-dev:ready" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "ready", "ready/version"),
    }, {
      updated_at = "2026-06-03T01:02:03Z",
    })
    local observed = run_observe(event, run_opts)
    issue_updated_at("2026-06-03T01:02:04Z")
    full_issue_view({ "fkst-dev:blocked" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "blocked", "blocked/version"),
    }, {
      updated_at = "2026-06-03T01:02:04Z",
    })
    local opened = run_open_pr(event, run_opts)

    t.eq(observed.exit_code, 0)
    t.eq(opened.exit_code, 0)
    t.eq(#opened.raises, 0)
  end,

  test_same_consumer_retry_refetches_current_issue_truth = function()
    local run_opts = shared_opts("same-consumer-retry")
    local event = issue({ labels = { "fkst-dev:ready" }, updated_at = "2026-06-03T01:02:03Z" })
    full_issue_view({ "fkst-dev:ready" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "ready", "ready/version"),
    }, {
      updated_at = "2026-06-03T01:02:03Z",
    })
    local first = run_observe(event, run_opts)
    full_issue_view({ "fkst-dev:blocked" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "blocked", "blocked/version"),
    }, {
      updated_at = "2026-06-03T01:02:04Z",
    })
    local retry = run_observe(event, run_opts)

    t.eq(first.exit_code, 0)
    t.eq(retry.exit_code, 0)
    t.eq(#retry.raises, 0)
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
    local second = run_observe(issue({
      labels = { "fkst-dev:ready" },
      updated_at = "2026-06-03T01:02:04Z",
      view_cache_key = "github-proxy/view/owner/repo/issue/42/2026-06-03T01-02-04Z",
    }), run_opts)

    t.eq(first.exit_code, 0)
    t.eq(second.exit_code, 0)
  end,

  test_pr_entity_view_refetches_same_consumer_retry = function()
    local run_opts = shared_opts("pr-same-consumer-retry")
    local event = {
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      title = "Bridge PR",
      url = "https://github.example/owner/repo/pull/7",
      state = "OPEN",
      updated_at = "2026-06-03T02:03:04Z",
      labels = {},
      dedup_key = "owner/repo#pr#7@2026-06-03T02:03:04Z",
      view_cache_key = "github-proxy/view/owner/repo/pr/7/2026-06-03T02-03-04Z",
      source_ref = h.pr_source_ref(),
    }

    h.mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", reviewing().version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", reviewing().version),
    }, "devloop-owner-repo-42-01HY", "def456", "OPEN", "dev")
    local first = run_observe_pr(event, run_opts)
    h.mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", reviewing().version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", reviewing().version),
    }, "devloop-owner-repo-42-01HY", "def456", "OPEN", "dev")
    local second = run_observe_pr(event, run_opts)

    t.eq(first.exit_code, 0)
    t.eq(second.exit_code, 0)
  end,
}
