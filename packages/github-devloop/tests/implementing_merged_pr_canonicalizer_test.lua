local entity_lib = require("devloop.entity")
local h = require("tests.devloop_helpers")
local entity_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")

local core = h.core
local t = h.t

local repo = "owner/repo"
local issue_number = 42
local pr_number = 7
local parent = "github-devloop/issue/owner/repo/42"
local child_pr = "github-devloop/pr/owner/repo/7"
local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local branch = "devloop-owner-repo-42-01HY"
local base_branch = "dev"
local head_sha = "0123456789abcdef0123456789abcdef01234567"
local merge_commit_sha = "1111111111111111111111111111111111111111"

local function comment(body, created_at)
  return {
    body = body,
    author_login = core._test_bot_login,
    created_at = created_at or "2026-06-03T01:00:00Z",
  }
end

local function parent_comments(state, extra_comments)
  local comments = {
    comment(core.state_marker(parent, state or "implementing", version), "2026-06-03T01:02:03Z"),
    comment(m_builders.pr_delegation_marker(parent, child_pr, pr_number, version, "g1"), "2026-06-03T01:03:03Z"),
  }
  for _, extra in ipairs(extra_comments or {}) do
    table.insert(comments, extra)
  end
  return comments
end

local function pr_comments()
  return {
    comment(m_builders.pr_origin_marker(parent, issue_number, branch, version, base_branch), "2026-06-03T01:04:03Z"),
  }
end

local function mock_env(write_mode)
  h.mock_bot_env()
  h.mock_write_env(write_mode or "")
  t.mock_command("gh api graphql", {
    stdout = '{"data":{"repository":{"issue":{"blockedBy":{"nodes":[]}}}}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function issue_fields(state, labels, extra_comments)
  return {
    repo = repo,
    number = issue_number,
    labels = labels or { "fkst-dev:enabled", "fkst-dev:implementing" },
    comments = parent_comments(state, extra_comments),
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
    times = 1,
    register_all_views = true,
  }
end

local function pr_fields(pr_state, merged_at)
  return {
    repo = repo,
    number = pr_number,
    comments = pr_comments(),
    head = branch,
    head_sha = head_sha,
    merge_commit_sha = merge_commit_sha,
    state = pr_state,
    merged_at = merged_at,
    base_branch = base_branch,
    labels = {},
    times = 1,
    register_all_views = true,
  }
end

local function mock_reads(pr_state, merged_at, state, labels, extra_comments)
  entity_mocks.mock_issue_read_forms(t, issue_fields(state, labels, extra_comments))
  entity_mocks.mock_pr_read_forms(t, pr_fields(pr_state, merged_at))
  entity_mocks.mock_pr_view_selector(t, pr_fields(pr_state, merged_at), entity_mocks.pr_origin_selector, 1)
end

local function run_pr_observe(pr_state, merged_at)
  mock_env()
  mock_reads(pr_state, merged_at)
  return t.run_department("departments/observe_issue/main.lua", {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "pr",
      repo = repo,
      number = pr_number,
      state = pr_state,
      updated_at = "2026-06-03T02:03:04Z",
      dedup_key = "owner/repo#pr#7@2026-06-03T02:03:04Z",
      source_ref = entity_lib.pr_source_ref(repo, pr_number),
    },
  }, h.opts("implementing-merged-pr-canonicalizer"))
end

local function run_issue_observe(pr_state, merged_at, state)
  mock_env()
  mock_reads(pr_state, merged_at, state)
  return t.run_department("departments/observe_issue/main.lua", {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "issue",
      repo = repo,
      number = issue_number,
      title = "Implement decision recorder",
      state = "OPEN",
      updated_at = "2026-06-03T02:03:04Z",
      dedup_key = "owner/repo#issue#42@2026-06-03T02:03:04Z",
      source_ref = entity_lib.issue_source_ref(repo, issue_number),
    },
  }, h.opts("implementing-merged-pr-canonicalizer-issue-poll"))
end

local function run_issue_close_poll(canonicalization_body)
  h.mock_bot_env()
  for _ = 1, 4 do
    h.mock_write_env("1")
  end
  t.mock_command("gh api graphql", {
    stdout = '{"data":{"repository":{"issue":{"blockedBy":{"nodes":[]}}}}}\n',
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
    stdout = base_branch,
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
    stdout = base_branch,
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh issue close", {
    stdout = "closed\n",
    stderr = "",
    exit_code = 0,
  })
  mock_reads("MERGED", "2026-06-03T02:05:04Z", "awaiting-pr", { "fkst-dev:enabled", "fkst-dev:awaiting-pr" }, {
    comment(canonicalization_body, "2026-06-03T02:06:04Z"),
  })
  return t.run_department("departments/observe_issue/main.lua", {
    queue = "devloop_observe_issue",
    payload = {
      schema = "github-proxy.v1",
      type = "issue",
      repo = repo,
      number = issue_number,
      title = "Implement decision recorder",
      state = "OPEN",
      updated_at = "2026-06-03T02:10:04Z",
      dedup_key = "owner/repo#issue#42@2026-06-03T02:10:04Z",
      source_ref = entity_lib.issue_source_ref(repo, issue_number),
    },
  }, h.opts("implementing-merged-pr-canonicalizer-close-poll"))
end

local function find_raise(raises, queue, predicate)
  for _, raised in ipairs(raises or {}) do
    if raised.queue == queue and (predicate == nil or predicate(raised.payload or {}, raised)) then
      return raised
    end
  end
  return nil
end

local function count_calls(needle)
  return h.count_calls(needle)
end

return {
  test_pr_entity_change_merged_child_canonicalizes_implementing_parent_to_awaiting_pr = function()
    local result = run_pr_observe("MERGED", "2026-06-03T02:05:04Z")

    t.eq(result.exit_code, 0)
    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request", function(payload)
      return tostring(payload.body or ""):find('state="awaiting-pr"', 1, true) ~= nil
    end)
    t.is_true(comment_raise ~= nil)
    t.is_true(tostring(comment_raise.payload.body):find("fkst:github-devloop:pr-delegation:v1", 1, true) ~= nil)
    t.is_true(tostring(comment_raise.payload.body):find('pr_proposal="' .. child_pr .. '"', 1, true) ~= nil)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request", function(payload)
      return payload.add_labels[1] == "fkst-dev:awaiting-pr"
    end)
    t.is_true(label_raise ~= nil)
    t.eq(count_calls("gh issue close 42 --repo owner/repo"), 0)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
  end,

  test_issue_poll_merged_child_canonicalizes_implementing_parent_then_parent_poll_closes = function()
    local result = run_issue_observe("MERGED", "2026-06-03T02:05:04Z")

    t.eq(result.exit_code, 0)
    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request", function(payload)
      return tostring(payload.body or ""):find('state="awaiting-pr"', 1, true) ~= nil
    end)
    t.is_true(comment_raise ~= nil)
    t.is_true(tostring(comment_raise.payload.body):find("fkst:github-devloop:pr-delegation:v1", 1, true) ~= nil)
    t.is_true(tostring(comment_raise.payload.body):find('pr_proposal="' .. child_pr .. '"', 1, true) ~= nil)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)

    local close_result = run_issue_close_poll(comment_raise.payload.body)
    t.eq(close_result.exit_code, 0)
    local close_comment = find_raise(close_result.raises, "github-proxy.github_issue_comment_request", function(payload)
      return tostring(payload.body or ""):find('state="merged"', 1, true) ~= nil
    end)
    t.is_true(close_comment ~= nil)
    t.eq(count_calls("gh issue close 42 --repo owner/repo"), 1)
  end,

  test_issue_poll_open_child_with_json_null_merged_at_does_not_canonicalize_parent = function()
    local result = run_issue_observe("OPEN", nil)

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_comment_request"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
    t.eq(count_calls("gh issue close 42 --repo owner/repo"), 0)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
  end,

  test_pr_entity_change_open_child_with_json_null_merged_at_does_not_canonicalize_parent = function()
    local result = run_pr_observe("OPEN", nil)

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_comment_request"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
    t.eq(count_calls("gh issue close 42 --repo owner/repo"), 0)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
  end,
}
