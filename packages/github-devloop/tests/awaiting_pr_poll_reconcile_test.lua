local h = require("tests.devloop_helpers")
local entity_mocks = require("tests.entity_read_mock_helpers")
local core = h.core
local t = h.t

local repo = "owner/repo"
local issue_number = 42
local pr_number = 7
local parent = "github-devloop/issue/owner/repo/42"
local child_pr = "github-devloop/pr/owner/repo/7"
local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local delegation = "g1"
local head_sha = "0123456789abcdef0123456789abcdef01234567"

local function comment(body, author, created_at)
  return {
    id = tostring(created_at or body):gsub("[^%w_%-]", "_"):sub(1, 60),
    body = body,
    author_login = author or core._test_bot_login,
    created_at = created_at or "2026-06-03T01:00:00Z",
  }
end

local function find_raise(raises, queue, predicate)
  for _, raised in ipairs(raises or {}) do
    if raised.queue == queue and (predicate == nil or predicate(raised.payload or {}, raised)) then
      return raised
    end
  end
  return nil
end

local function count_raises(raises, queue)
  local count = 0
  for _, raised in ipairs(raises or {}) do
    if raised.queue == queue then
      count = count + 1
    end
  end
  return count
end

local function count_calls(needle)
  return h.count_calls(needle)
end

local function mock_issue_close()
  t.mock_command("gh issue close", {
    stdout = "closed\n",
    stderr = "",
    exit_code = 0,
  })
end

local function parent_comments(fields)
  local f = fields or {}
  local state = f.state or "awaiting-pr"
  local state_version = f.version or version
  local comments = {
    comment(core.state_marker(parent, state, state_version), core._test_bot_login, "2026-06-03T01:02:03Z"),
  }
  if f.delegation ~= false then
    table.insert(comments, comment(core.pr_delegation_marker(
      f.parent or parent,
      f.child or child_pr,
      f.pr_number or pr_number,
      f.delegation_version or state_version,
      f.delegation_generation or delegation
    ), core._test_bot_login, "2026-06-03T01:03:03Z"))
  end
  return comments
end

local function child_comments(state, child_version)
  return {
    comment(core.state_marker(parent, state, child_version or version), core._test_bot_login, "2026-06-03T01:04:03Z"),
  }
end

local function child_merged_comments_with_kept_promotion()
  return {
    comment(core.state_marker(parent, "merged", version)
      .. "\n" .. core.merged_marker(parent, pr_number, version, head_sha), core._test_bot_login, "2026-06-03T01:04:03Z"),
  }
end

local function mock_env()
  h.mock_bot_env()
  h.mock_write_env("")
  t.mock_command("gh api graphql", {
    stdout = '{"data":{"repository":{"issue":{"blockedBy":{"nodes":[]}}}}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_real_write_env()
  h.mock_bot_env()
  for _ = 1, 4 do
    h.mock_write_env("1")
  end
  t.mock_command("gh api graphql", {
    stdout = '{"data":{"repository":{"issue":{"blockedBy":{"nodes":[]}}}}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_reads(issue_comments, pr_comments, opts)
  local options = opts or {}
  entity_mocks.mock_issue_view_selector(t, {
    repo = repo,
    number = issue_number,
    labels = options.labels or { "fkst-dev:enabled", "fkst-dev:awaiting-pr" },
    comments = issue_comments,
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "title,body,comments,labels,state,updatedAt,assignees,author")
  entity_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = options.pr_number or pr_number,
    comments = pr_comments,
    head = "devloop-owner-repo-42-01HY",
    head_sha = head_sha,
    state = options.pr_state or "OPEN",
    base_branch = "dev",
    labels = {},
  }, entity_mocks.pr_origin_selector)
end

local function run_observe(issue_comments, pr_comments, opts)
  local options = opts or {}
  if options.write == "real" then
    mock_real_write_env()
  else
    mock_env()
  end
  mock_reads(issue_comments, pr_comments, options)
  return t.run_department("departments/observe_issue/main.lua", {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "issue",
      repo = repo,
      number = issue_number,
      title = "Implement decision recorder",
      state = "OPEN",
      updated_at = "2026-06-03T01:02:03Z",
      labels = { "fkst-dev:enabled", "fkst-dev:awaiting-pr" },
      dedup_key = "owner/repo#issue#42@2026-06-03T01:02:03Z",
      source_ref = core.issue_source_ref(repo, issue_number),
    },
  })
end

local function resume_comment(result)
  return find_raise(result.raises, "github-proxy.github_issue_comment_request")
end

return {
  test_child_merged_reconciles_parent_to_merged = function()
    mock_issue_close()
    local result = run_observe(parent_comments(), child_comments("merged"), { write = "real" })

    t.eq(result.exit_code, 0)
    local resume = resume_comment(result)
    t.is_true(resume ~= nil)
    t.is_true(resume.payload.body:find('state="merged"', 1, true) ~= nil)
    t.eq(count_raises(result.raises, "github-proxy.github_issue_label_request"), 1)
    t.eq(count_calls("gh issue close 42 --repo owner/repo"), 1)
  end,

  test_child_merged_with_kept_issue_promotion_closes_issue_once = function()
    mock_issue_close()
    local result = run_observe(parent_comments(), child_merged_comments_with_kept_promotion(), { write = "real" })

    t.eq(result.exit_code, 0)
    local resume = resume_comment(result)
    t.is_true(resume ~= nil)
    t.is_true(resume.payload.body:find('state="merged"', 1, true) ~= nil)
    t.eq(count_calls("gh issue close 42 --repo owner/repo"), 1)
  end,

  test_child_closed_unmerged_reconciles_parent_to_ready_generation = function()
    local result = run_observe(parent_comments(), child_comments("closed-unmerged"))

    t.eq(result.exit_code, 0)
    local resume = resume_comment(result)
    t.is_true(resume ~= nil)
    t.is_true(resume.payload.body:find('state="ready"', 1, true) ~= nil)
    t.is_true(resume.payload.body:find("/reimplement/1", 1, true) ~= nil)
  end,

  test_child_closed_unmerged_blocks_at_reimplementation_budget = function()
    local exhausted = version .. "/reimplement/12"
    local result = run_observe(
      parent_comments({ version = exhausted, delegation_version = exhausted }),
      child_comments("closed-unmerged", exhausted)
    )

    t.eq(result.exit_code, 0)
    local resume = resume_comment(result)
    t.is_true(resume ~= nil)
    t.is_true(resume.payload.body:find('state="blocked"', 1, true) ~= nil)
    t.is_true(resume.payload.body:find("replacement-budget-exhausted", 1, true) ~= nil)
  end,

  test_child_blocked_reconciles_parent_to_blocked = function()
    local result = run_observe(parent_comments(), child_comments("blocked"))

    t.eq(result.exit_code, 0)
    local resume = resume_comment(result)
    t.is_true(resume ~= nil)
    t.is_true(resume.payload.body:find('state="blocked"', 1, true) ~= nil)
    t.is_true(resume.payload.body:find("child-pr-blocked", 1, true) ~= nil)
  end,

  test_child_nonterminal_defers_without_parent_cas = function()
    local result = run_observe(parent_comments(), child_comments("merge-ready"))

    t.eq(result.exit_code, 0)
    t.eq(count_raises(result.raises, "github-proxy.github_issue_comment_request"), 0)
    t.eq(count_raises(result.raises, "github-proxy.github_issue_label_request"), 0)
  end,

  test_missing_delegation_fails_closed_without_stale_cas = function()
    local result = run_observe(parent_comments({ delegation = false }), child_comments("merged"))

    t.eq(result.exit_code, 0)
    t.eq(count_raises(result.raises, "github-proxy.github_issue_comment_request"), 0)
  end,

  test_stale_generation_delegation_fails_closed_without_stale_cas = function()
    local result = run_observe(
      parent_comments({ delegation_version = version .. "/old" }),
      child_comments("merged")
    )

    t.eq(result.exit_code, 0)
    t.eq(count_raises(result.raises, "github-proxy.github_issue_comment_request"), 0)
  end,

  test_idempotent_repoll_after_parent_transition_is_noop = function()
    local close_calls_before = count_calls("gh issue close 42 --repo owner/repo")
    local result = run_observe(parent_comments({ state = "merged" }), child_comments("merged"), {
      labels = { "fkst-dev:enabled", "fkst-dev:merged" },
      write = "real",
    })

    t.eq(result.exit_code, 0)
    t.eq(count_raises(result.raises, "github-proxy.github_issue_comment_request"), 0)
    t.eq(count_raises(result.raises, "github-proxy.github_issue_label_request"), 0)
    t.eq(count_calls("gh issue close 42 --repo owner/repo"), close_calls_before)
  end,
}
