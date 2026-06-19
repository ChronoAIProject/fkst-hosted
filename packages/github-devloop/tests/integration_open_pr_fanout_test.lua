local h = require("tests.devloop_helpers")
require("tests.cache_seed_helpers")
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
  entity_read_mocks.mock_issue_read_forms(t, {
    title = fields.title or "Implement decision recorder",
    body = fields.body or "",
    state = fields.state or "OPEN",
    updated_at = fields.updated_at or "2026-06-03T01:02:03Z",
    labels = labels,
    comments = comments,
    assignees = { fields.assignee_login or "fkst-test-bot" },
    author_login = fields.author_login or "fkst-test-bot",
    times = fields.times,
  })
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

local function shell_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function issue_rest_command(repo, number)
  return "gh api repos/" .. tostring(repo) .. "/issues/" .. tostring(number)
end

local function comments_rest_command(repo, number)
  return "gh api --paginate --slurp "
    .. "repos/" .. tostring(repo) .. "/issues/" .. tostring(number) .. "/comments?per_page=100"
end

local function issue_claim_command(repo, number)
  return "gh issue view " .. tostring(number)
    .. " --repo " .. tostring(repo)
    .. " --json"
end

local function count_exact_calls(command)
  local count = 0
  local read_prefix = "GH_TOKEN=${FKST_GITHUB_READ_TOKEN:-} GITHUB_TOKEN=${FKST_GITHUB_READ_TOKEN:-} "
  local expected = h.argv_rendered(command)
  for _, call in ipairs(t.command_calls()) do
    local rendered = tostring(call.rendered or "")
    if rendered:sub(1, #read_prefix) == read_prefix then
      rendered = rendered:sub(#read_prefix + 1)
    end
    local normalized = h.argv_rendered(rendered)
    if normalized == expected then
      count = count + 1
    end
  end
  return count
end

local function encode_json_string(value)
  return tostring(value or "")
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\b", "\\b")
    :gsub("\f", "\\f")
    :gsub("\n", "\\n")
    :gsub("\r", "\\r")
    :gsub("\t", "\\t")
end

local function seed_cached_issue_view(repo, issue_number, stdout, updated_at, producer)
  return {
    key = core.entity_view_cache_key(repo, "issue", issue_number),
    value = '{"updated_at":"' .. encode_json_string(updated_at)
    .. '","producer":"' .. encode_json_string(producer or "seed")
    .. '","stdout":"' .. encode_json_string(stdout)
    .. '"}',
  }
end

local function seed_cache(entry, run_opts)
  return t.run_department("departments/test_cache_seed/main.lua", {
    queue = "cache_seed",
    payload = entry,
  }, run_opts)
end

local function assert_clean_open_pr_skip(result)
  t.eq(result.exit_code, 0)
  t.eq(#result.raises, 0)
  t.eq(count_calls("show-ref --verify --quiet"), 0)
  t.eq(count_calls("rev-parse --verify"), 0)
  t.eq(count_calls("git -C"), 0)
end

local function assert_missing_implementing_fact_defer(result)
  t.eq(result.exit_code, 0)
  t.eq(#result.raises, 0)
  t.eq(count_calls("show-ref --verify --quiet"), 0)
  t.eq(count_calls("rev-parse --verify"), 0)
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

  test_open_pr_entity_change_marks_non_descendant_head_impl_failed = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_issue_open_pr({ "fkst-dev:implementing" }, {
      core.state_marker(proposal_id, "implementing", impl_version),
      core.implementing_marker(proposal_id, impl_version, "devloop-owner-repo-42-01HY", "abc123", "dev", "abc123"),
    })
    mock_branch_exists("devloop-owner-repo-42-01HY", "def456")
    mock_branch_head_descends(false)
    mock_bot_env()

    local result = run_open_pr(issue({ labels = { "fkst-dev:implementing" } }), opts("open-pr-non-descendant-head", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment.payload.body:find(core.state_marker(proposal_id, "impl-failed", impl_version), 1, true) ~= nil)
    t.is_true(comment.payload.body:find(core.impl_failure_marker(proposal_id, impl_version, "non-descendant-head"), 1, true) ~= nil)
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label.payload.add_labels[1], "fkst-dev:impl-failed")
    t.eq(find_raise(result.raises, "github-proxy.github_pr_open_request"), nil)
    t.eq(count_calls("show-ref --verify --quiet"), 1)
    t.eq(count_calls("rev-parse --verify"), 1)
    t.eq(count_calls("merge-base --is-ancestor"), 1)
  end,

  test_open_pr_liveness_entity_change_for_implementing_no_pr_routes_to_impl_failed = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local proposal_id = "github-devloop/issue/owner/repo/42"
    mock_issue_open_pr({ "fkst-dev:implementing" }, {
      core.state_marker(proposal_id, "implementing", impl_version),
      core.implementing_marker(proposal_id, impl_version, "devloop-owner-repo-42-01HY", "abc123", "dev", "abc123"),
    })
    mock_branch_exists("devloop-owner-repo-42-01HY", "def456")
    mock_branch_head_descends(false)
    mock_bot_env()

    local result = run_open_pr(issue({
      labels = { "fkst-dev:implementing" },
      source = "liveness-scan",
      dedup_key = "liveness-scan/owner/repo/issue/42/2026-06-03T01:02:03Z/735",
    }), opts("open-pr-liveness-implementing-no-pr", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment.payload.body:find(core.state_marker(proposal_id, "impl-failed", impl_version), 1, true) ~= nil)
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label.payload.add_labels[1], "fkst-dev:impl-failed")
    t.eq(find_raise(result.raises, "github-proxy.github_pr_open_request"), nil)
    t.eq(count_calls("show-ref --verify --quiet"), 1)
    t.eq(count_calls("rev-parse --verify"), 1)
    t.eq(count_calls("merge-base --is-ancestor"), 1)
  end,

  test_open_pr_redrive_leaves_pr_open_state_label_to_observe_issue = function()
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
    t.eq(#result.raises, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
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

  test_open_pr_poll_defers_when_implementing_fact_marker_missing = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_issue_open_pr({ "fkst-dev:implementing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "implementing", impl_version),
    })

    local result = run_open_pr(issue({ labels = { "fkst-dev:implementing" } }), opts("open-pr-poll-missing-implementing-fact"))

    assert_missing_implementing_fact_defer(result)
  end,

  test_open_pr_direct_defers_when_implementing_fact_marker_missing = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local event = core.build_devloop_open_pr_payload("owner/repo", 42, {
      proposal_id = "github-devloop/issue/owner/repo/42",
      dedup_key = impl_version,
      source_ref = source_ref(),
    }, "devloop-owner-repo-42-01HY", "abc123", "dev")
    mock_issue_open_pr({ "fkst-dev:implementing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "implementing", impl_version),
    })

    local result = run_open_pr(event, opts("open-pr-direct-missing-implementing-fact"))

    assert_missing_implementing_fact_defer(result)
  end,

  test_open_pr_poll_recovers_after_direct_missing_fact_defer = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local direct_event = core.build_devloop_open_pr_payload("owner/repo", 42, {
      proposal_id = proposal_id,
      dedup_key = impl_version,
      source_ref = source_ref(),
    }, "devloop-owner-repo-42-01HY", "abc123", "dev")
    local run_opts = opts("open-pr-direct-defer-then-poll-recover", {
      FKST_GITHUB_WRITE = "1",
    })

    mock_issue_open_pr({ "fkst-dev:implementing" }, {
      core.state_marker(proposal_id, "implementing", impl_version),
    }, {
      updated_at = "2026-06-03T01:02:03Z",
      times = 1,
    })
    local deferred = run_open_pr(direct_event, run_opts)

    assert_missing_implementing_fact_defer(deferred)

    mock_issue_open_pr({ "fkst-dev:implementing" }, {
      core.state_marker(proposal_id, "implementing", impl_version),
      core.implementing_marker(proposal_id, impl_version, "devloop-owner-repo-42-01HY", "abc123", "dev", "abc123"),
    }, {
      updated_at = "2026-06-03T01:02:34Z",
    })
    mock_branch_exists("devloop-owner-repo-42-01HY", "abc123")
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    local recovered = run_open_pr(issue({
      labels = { "fkst-dev:implementing" },
      updated_at = "2026-06-03T01:02:34Z",
      dedup_key = "owner/repo#issue#42@2026-06-03T01:02:34Z",
    }), run_opts)

    t.eq(recovered.exit_code, 0)
    t.eq(#recovered.raises, 1)
    local pr_raise = find_raise(recovered.raises, "github-proxy.github_pr_open_request")
    t.eq(pr_raise.payload.schema, "github-proxy.pr-open.v1")
    t.eq(pr_raise.payload.branch, "devloop-owner-repo-42-01HY")
    t.eq(pr_raise.payload.head_sha, "abc123")
    t.eq(pr_raise.payload.impl_version, impl_version)
    t.eq(count_calls("show-ref --verify --quiet"), 1)
    t.eq(count_calls("rev-parse --verify"), 1)
    t.eq(count_calls("merge-base --is-ancestor"), 0)
  end,

  test_observe_claim_acquire_read_bypasses_same_validator_cache = function()
    local run_opts = shared_opts("observe-claim-force-fresh")
    local updated_at = "2026-06-03T01:02:03Z"
    local rest_command = issue_rest_command("owner/repo", 42)
    local comments_command = comments_rest_command("owner/repo", 42)
    local claim_command = issue_claim_command("owner/repo", 42)
    seed_cache(seed_cached_issue_view("owner/repo", 42, entity_read_mocks.issue_view_stdout({
      repo = "owner/repo",
      number = 42,
      labels = { "fkst-dev:enabled" },
      updated_at = updated_at,
      assignees = { "other-bot" },
      author_login = "fkst-test-bot",
    }), updated_at, "stale-claim"), run_opts)
    entity_read_mocks.mock_issue_read_forms(t, {
      repo = "owner/repo",
      number = 42,
      labels = { "fkst-dev:enabled" },
      comments = {},
      updated_at = updated_at,
      assignees = {},
      author_login = "fkst-test-bot",
      register_all_views = true,
      times = 1,
    })

    local result = run_observe(issue({
      labels = { "fkst-dev:enabled" },
      updated_at = updated_at,
    }), run_opts)

    t.eq(result.exit_code, 0)
    t.eq(count_exact_calls(rest_command), 1)
    t.eq(count_exact_calls(comments_command), 1)
    t.eq(count_calls(claim_command), 1)
  end,

  test_observe_marker_idempotency_read_bypasses_same_validator_cache = function()
    local run_opts = shared_opts("observe-marker-force-fresh")
    local updated_at = "2026-06-03T01:02:03Z"
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local rest_command = issue_rest_command("owner/repo", 42)
    local comments_command = comments_rest_command("owner/repo", 42)
    local claim_command = issue_claim_command("owner/repo", 42)
    seed_cache(seed_cached_issue_view("owner/repo", 42, entity_read_mocks.issue_view_stdout({
      repo = "owner/repo",
      number = 42,
      labels = { "fkst-dev:enabled" },
      comments = {},
      updated_at = updated_at,
      assignees = { "fkst-test-bot" },
      author_login = "fkst-test-bot",
    }), updated_at, "stale-marker"), run_opts)
    entity_read_mocks.mock_issue_read_forms(t, {
      repo = "owner/repo",
      number = 42,
      labels = { "fkst-dev:thinking" },
      comments = {
        core.state_marker(proposal_id, "thinking", "owner/repo#issue#42@2026-06-03T01:02:03Z"),
      },
      updated_at = updated_at,
      assignees = { "fkst-test-bot" },
      author_login = "fkst-test-bot",
      register_all_views = true,
      times = 1,
    })

    local result = run_observe(issue({
      labels = { "fkst-dev:enabled" },
      updated_at = updated_at,
    }), run_opts)

    t.eq(result.exit_code, 0)
    t.eq(count_exact_calls(rest_command), 1)
    t.eq(count_exact_calls(comments_command), 1)
    t.eq(count_calls(claim_command), 0)
  end,

  test_open_pr_write_gate_claim_read_bypasses_same_validator_cache = function()
    local run_opts = opts("entity-view-cache-open-pr-gate-force-fresh", {
      FKST_GITHUB_WRITE = "1",
    })
    local updated_at = "2026-06-03T01:02:03Z"
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local rest_command = issue_rest_command("owner/repo", 42)
    local comments_command = comments_rest_command("owner/repo", 42)
    local claim_command = issue_claim_command("owner/repo", 42)
    seed_cache(seed_cached_issue_view("owner/repo", 42, entity_read_mocks.issue_view_stdout({
      repo = "owner/repo",
      number = 42,
      labels = { "fkst-dev:implementing" },
      comments = {
        core.state_marker("github-devloop/issue/owner/repo/42", "implementing", impl_version),
        core.implementing_marker("github-devloop/issue/owner/repo/42", impl_version, "devloop-owner-repo-42-01HY", "abc123", "dev", "abc123"),
      },
      updated_at = updated_at,
      assignees = { "other-bot" },
      author_login = "fkst-test-bot",
    }), updated_at, "stale-open-pr"), run_opts)
    mock_issue_open_pr({ "fkst-dev:implementing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "implementing", impl_version),
      core.implementing_marker("github-devloop/issue/owner/repo/42", impl_version, "devloop-owner-repo-42-01HY", "abc123", "dev", "abc123"),
    }, {
      updated_at = updated_at,
      assignees = { "fkst-test-bot" },
      author_login = "fkst-test-bot",
    })
    mock_branch_exists("devloop-owner-repo-42-01HY", "abc123")
    mock_bot_env()
    mock_write_env("1")

    local result = run_open_pr(issue({
      labels = { "fkst-dev:implementing" },
      updated_at = updated_at,
    }), run_opts)

    t.eq(result.exit_code, 0)
    t.eq(count_exact_calls(rest_command), 1)
    t.eq(count_exact_calls(comments_command), 1)
    t.eq(count_calls(claim_command), 0)
  end,

  test_validated_issue_view_is_fresh_across_event_driven_departments = function()
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
