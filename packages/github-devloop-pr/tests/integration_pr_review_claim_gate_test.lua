local m_claims = require("devloop.claims")
local h = require("tests.devloop_helpers")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")
local t = h.t
local core = h.core
local opts = h.opts
local reviewing = h.reviewing
local run_observe_pr = h.run_observe_pr
local run_review_pr = h.run_review_pr
local mock_bot_env = h.mock_bot_env
local mock_issue_reviewing = h.mock_issue_reviewing
local mock_issue_review = h.mock_issue_review
local mock_pr_origin = h.mock_pr_origin
local mock_pr_origin_sequence = h.mock_pr_origin_sequence
local count_calls = h.count_calls

local function pr_event()
  return {
    schema = "github-proxy.v1",
    type = "pr",
    repo = "owner/repo",
    number = 7,
    dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
    source_ref = {
      kind = "external",
      ref = "owner/repo#pr/7",
    },
  }
end

local function origin_marker(version)
  return m_builders.pr_origin_marker("github-devloop/issue/owner/repo/42",
    "42",
    "devloop-owner-repo-42-01HY",
    version,
    "dev"
  )
end

local function review_state_marker(version)
  return core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", version)
end

local function pr_open_state_marker(version)
  return core.state_marker("github-devloop/issue/owner/repo/42", "pr-open", version)
end

local function unmanaged_origin_marker(version, base_branch)
  return m_builders.pr_origin_marker("github-devloop/issue/owner/repo/42",
    "42",
    "devloop-owner-repo-42-01HY",
    version,
    base_branch or "integration"
  )
end

local function unmanaged_comment_raise(result)
  return h.find_raise(result.raises, "github-proxy.github_pr_comment_request", function(payload)
    return tostring(payload.body or ""):find("pr-base-unmanaged", 1, true) ~= nil
  end)
end

local function mock_observe_pr_env(integration_branch)
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
    stdout = "fkst-test-bot",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
    stdout = "fkst-test-bot",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
    stdout = "dev",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
    stdout = tostring(integration_branch or ""),
    stderr = "",
    exit_code = 0,
  })
end

local function run_observe_pr_with_integration(event, name, integration_branch)
  mock_observe_pr_env(integration_branch)
  return t.run_department("departments/observe_pr/main.lua", {
    queue = "github-proxy.github_entity_changed",
    payload = event,
  }, opts(name))
end

local function mock_precise_pr_origin(fields)
  local f = fields or {}
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = "owner/repo",
    number = 7,
    comments = f.comments,
    head = f.head or "devloop-owner-repo-42-01HY",
    head_sha = f.head_sha or "def456",
    state = f.state or "OPEN",
    base_branch = f.base_branch or "integration",
    labels = f.labels or {},
  }, entity_read_mocks.pr_origin_selector)
end

return {
  test_verify_pr_review_issue_claim_accepts_unassigned_self_author = function()
    mock_bot_env()
    local ok = m_claims.verify_pr_review_issue_claim("claim-test", "owner/repo", 42, {
      assignees = {},
      author_login = "fkst-test-bot",
    }, "github-devloop/issue/owner/repo/42")
    t.eq(ok, true)
  end,

  test_observe_pr_skips_issue_backed_review_when_claim_is_other = function()
    local impl_version = reviewing().version
    mock_bot_env()
    mock_pr_origin({ origin_marker(impl_version) })
    mock_issue_reviewing({ "fkst-dev:pr-open" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "pr-open", impl_version),
    }, {
      assignees = { "other-bot" },
    })

    local result = run_observe_pr(pr_event(), opts("observe-pr-other-claim"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_observe_pr_processes_unassigned_self_authored_backing_issue = function()
    local impl_version = reviewing().version
    mock_bot_env()
    mock_pr_origin({ origin_marker(impl_version) })
    mock_issue_reviewing({ "fkst-dev:pr-open" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "pr-open", impl_version),
    }, {
      assignees = {},
      author_login = "fkst-test-bot",
    })

    local result = run_observe_pr(pr_event(), opts("observe-pr-unassigned-self-author"))
    t.eq(result.exit_code, 0)
    t.is_true(h.find_causal_raise(result, "devloop_reviewing") ~= nil)
  end,

  test_observe_pr_skips_without_backing_issue = function()
    mock_bot_env()
    mock_pr_origin({}, "feature-branch", "def456")

    local result = run_observe_pr(pr_event(), opts("observe-pr-no-backing-issue"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_observe_pr_claim_read_failure_fails_closed = function()
    local impl_version = reviewing().version
    mock_bot_env()
    mock_pr_origin({ origin_marker(impl_version) })
    t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
      stdout = "",
      stderr = "forced claim failure",
      exit_code = 1,
    })

    local result = run_observe_pr(pr_event(), opts("observe-pr-claim-read-fails"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_review_pr_skips_expensive_review_when_claim_is_other = function()
    local event = reviewing()
    mock_bot_env()
    mock_issue_review({ "fkst-dev:reviewing" }, {
      review_state_marker(event.version),
    }, {
      assignees = { "other-bot" },
    })
    mock_pr_origin_sequence({
      { comments = { origin_marker(event.version) }, head = "devloop-owner-repo-42-01HY", head_sha = "def456" },
    })

    local result = run_review_pr(event, opts("review-pr-other-claim"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh pr diff"), 0)
  end,

  test_observe_pr_blocks_self_claimed_pr_open_when_base_is_unmanaged = function()
    local impl_version = reviewing().version
    mock_bot_env()
    mock_pr_origin({
      unmanaged_origin_marker(impl_version, "feature-unmanaged"),
      pr_open_state_marker(impl_version),
    }, "devloop-owner-repo-42-01HY", "def456", "OPEN", "feature-unmanaged")
    mock_issue_reviewing({ "fkst-dev:pr-open" }, {
      pr_open_state_marker(impl_version),
    }, {
      assignees = { "fkst-test-bot" },
    })

    local result = run_observe_pr(pr_event(), opts("observe-pr-self-unmanaged-base"))
    t.eq(result.exit_code, 0)

    local comment = unmanaged_comment_raise(result)
    t.is_true(comment ~= nil)
    t.is_true(comment.payload.body:find(core.state_marker("github-devloop/issue/owner/repo/42", "blocked", impl_version .. "/blocked/pr-base-unmanaged"), 1, true) ~= nil)
    t.is_true(comment.payload.body:find('reason="pr-base-unmanaged"', 1, true) ~= nil)
    t.is_true(comment.payload.body:find('pr_base="feature-unmanaged"', 1, true) ~= nil)
    t.is_true(comment.payload.body:find('integration_branch="dev"', 1, true) ~= nil)

    local issue_label = h.find_raise(result.raises, "github-proxy.github_issue_label_request", function(payload)
      return tostring(payload.target_kind or "issue") == "issue"
    end)
    t.eq(issue_label, nil)
    local label = h.find_raise(h.run_comment_handoff_from_request(
      comment.payload,
      "IC_unmanaged_base_blocked_1",
      "observe-pr-self-unmanaged-base-handoff"
    ).raises, "github-proxy.github_issue_label_request", function(payload)
      return tostring(payload.target_kind or "issue") == "pr"
    end)
    t.eq(label.payload.add_labels[1], "fkst-dev:blocked")
    t.eq(label.payload.target_number, 7)
    t.eq(h.find_raise(result.raises, "devloop_reviewing"), nil)
  end,

  test_observe_pr_self_heals_spurious_unmanaged_base_block_after_integration_branch_recovers = function()
    local impl_version = reviewing().version
    local blocked_version = impl_version .. "/blocked/pr-base-unmanaged"
    local blocked_marker = core.state_marker("github-devloop/issue/owner/repo/42", "blocked", blocked_version)

    mock_precise_pr_origin({
      comments = {
        unmanaged_origin_marker(impl_version, "integration"),
        pr_open_state_marker(impl_version),
      },
      base_branch = "integration",
    })
    mock_issue_reviewing({ "fkst-dev:pr-open" }, {
      pr_open_state_marker(impl_version),
    }, {
      assignees = { "fkst-test-bot" },
    })

    local first = run_observe_pr_with_integration(pr_event(), "observe-pr-two-pass-wrong-integration", "dev")
    t.eq(first.exit_code, 0)

    local blocked_comment = unmanaged_comment_raise(first)
    t.is_true(blocked_comment ~= nil)
    t.is_true(blocked_comment.payload.body:find(core.state_marker("github-devloop/issue/owner/repo/42", "blocked", blocked_version), 1, true) ~= nil)
    t.is_true(blocked_comment.payload.body:find('pr_base="integration"', 1, true) ~= nil)
    t.is_true(blocked_comment.payload.body:find('integration_branch="dev"', 1, true) ~= nil)
    t.eq(h.find_raise(first.raises, "devloop_reviewing"), nil)

    local blocked_handoff = h.run_comment_handoff_from_request(
      blocked_comment.payload,
      "IC_two_pass_unmanaged_base_blocked_1",
      "observe-pr-two-pass-blocked-handoff"
    )
    local blocked_label = h.find_raise(blocked_handoff.raises, "github-proxy.github_issue_label_request", function(payload)
      return tostring(payload.target_kind or "issue") == "pr"
    end)
    t.eq(blocked_label.payload.add_labels[1], "fkst-dev:blocked")

    mock_precise_pr_origin({
      comments = {
        unmanaged_origin_marker(impl_version, "integration"),
        pr_open_state_marker(impl_version),
        blocked_marker,
      },
      base_branch = "integration",
      labels = { "fkst-dev:blocked" },
    })
    mock_issue_reviewing({ "fkst-dev:pr-open" }, {
      pr_open_state_marker(impl_version),
    }, {
      assignees = { "fkst-test-bot" },
    })

    local second = run_observe_pr_with_integration(pr_event(), "observe-pr-two-pass-correct-integration", "integration")
    t.eq(second.exit_code, 0)
    t.eq(unmanaged_comment_raise(second), nil)

    local reviewing_comment = h.find_raise(second.raises, "github-proxy.github_pr_comment_request", function(payload)
      return payload.handoff ~= nil and payload.handoff.kind == "github-devloop.reviewing"
    end)
    t.is_true(reviewing_comment ~= nil)
    t.eq(reviewing_comment.payload.handoff.version, core.next_review_loop_version(impl_version))
    t.is_true(reviewing_comment.payload.body:find(core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", core.next_review_loop_version(impl_version)), 1, true) ~= nil)

    local reviewing_raise = h.find_causal_raise(second, "devloop_reviewing")
    t.is_true(reviewing_raise ~= nil)
    t.eq(reviewing_raise.payload.version, core.next_review_loop_version(impl_version))
  end,

  test_observe_pr_leaves_foreign_claimed_unmanaged_base_untouched = function()
    local impl_version = reviewing().version
    mock_bot_env()
    mock_pr_origin({
      unmanaged_origin_marker(impl_version, "integration"),
      pr_open_state_marker(impl_version),
    }, "devloop-owner-repo-42-01HY", "def456", "OPEN", "integration")
    mock_issue_reviewing({ "fkst-dev:pr-open" }, {
      pr_open_state_marker(impl_version),
    }, {
      assignees = { "other-bot" },
    })

    local result = run_observe_pr(pr_event(), opts("observe-pr-foreign-unmanaged-base"))
    t.eq(result.exit_code, 0)
    t.eq(unmanaged_comment_raise(result), nil)
    t.eq(h.find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
    t.eq(h.find_raise(result.raises, "devloop_reviewing"), nil)
  end,

  test_observe_pr_base_matched_self_claimed_pr_open_still_reviews = function()
    local impl_version = reviewing().version
    mock_bot_env()
    mock_pr_origin({
      unmanaged_origin_marker(impl_version, "dev"),
      pr_open_state_marker(impl_version),
    }, "devloop-owner-repo-42-01HY", "def456", "OPEN", "dev")
    mock_issue_reviewing({ "fkst-dev:pr-open" }, {
      pr_open_state_marker(impl_version),
    }, {
      assignees = { "fkst-test-bot" },
    })

    local result = run_observe_pr(pr_event(), opts("observe-pr-self-base-matched"))
    t.eq(result.exit_code, 0)
    t.eq(unmanaged_comment_raise(result), nil)
    t.is_true(h.find_causal_raise(result, "devloop_reviewing") ~= nil)
  end,

}
