local h = require("tests.devloop_helpers")
local entity_mocks = require("tests.entity_read_mock_helpers")
local core = h.core
local t = h.t

local repo = "owner/repo"
local issue_number = 42
local pr_number = 7
local parent = "github-devloop/issue/owner/repo/42"
local child = "github-devloop/pr/owner/repo/7"
local version = "2026-06-03T01-02-03Z/implementing"
local delegation = "delegation-1"
local head_sha = "0123456789abcdef0123456789abcdef01234567"
local merge_sha = "abcdef0123456789abcdef0123456789abcdef01"

local function comment(body, author, created_at)
  return {
    id = tostring(created_at or "c1"),
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

local function count_raises(raises, queue, predicate)
  local count = 0
  for _, raised in ipairs(raises or {}) do
    if raised.queue == queue and (predicate == nil or predicate(raised.payload or {}, raised)) then
      count = count + 1
    end
  end
  return count
end

local function package_root()
  local source = package.searchpath("tests.pr_terminal_return_test", package.path)
  return source:match("(.+)/tests/pr_terminal_return_test%.lua$")
end

local function read_file(path)
  local handle = assert(io.open(path, "r"))
  local body = handle:read("*a")
  handle:close()
  return body
end

local function terminal_marker(fields)
  local terminal = fields or {}
  return core.pr_terminal_marker({
    terminal = terminal.terminal or "merged",
    pr_proposal = terminal.pr_proposal or child,
    repo = terminal.repo or repo,
    pr_identity = terminal.pr_identity or pr_number,
    pr_number = terminal.pr_number or pr_number,
    delegation_generation = terminal.delegation_generation or delegation,
    head_sha = terminal.head_sha or head_sha,
    merge_commit_sha = terminal.merge_commit_sha,
    terminal_marker_id = terminal.terminal_marker_id or "terminal-1",
  })
end

local function terminal_payload(fields)
  local terminal = fields or {}
  return core.build_devloop_pr_terminal_payload(
    terminal.proposal_id or parent,
    terminal.pr_number or pr_number,
    terminal.version or version,
    terminal.terminal or "merged",
    core.pr_source_ref(terminal.repo or repo, terminal.pr_number or pr_number),
    {
      terminal = terminal.terminal or "merged",
      pr_proposal = terminal.pr_proposal or child,
      repo = terminal.repo or repo,
      pr_identity = terminal.pr_identity or pr_number,
      delegation_generation = terminal.delegation_generation or delegation,
      head_sha = terminal.head_sha or head_sha,
      merge_commit_sha = terminal.merge_commit_sha,
      terminal_marker_id = terminal.terminal_marker_id or "terminal-1",
    }
  )
end

local function parent_comments(state, fields)
  local f = fields or {}
  local comments = {
    comment(core.state_marker(parent, state, f.version or version), core._test_bot_login, "2026-06-03T01:02:03Z"),
  }
  if f.delegation ~= false then
    table.insert(comments, comment(core.pr_delegation_marker(
      parent,
      f.pr_proposal or child,
      f.pr_number or pr_number,
      f.version or version,
      f.delegation_generation or delegation
    ), core._test_bot_login, "2026-06-03T01:03:03Z"))
  end
  if f.child_completed ~= nil then
    table.insert(comments, comment(f.child_completed, core._test_bot_login, "2026-06-03T01:04:03Z"))
  end
  return comments
end

local function mock_env()
  h.mock_bot_env()
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "", stderr = "", exit_code = 0 })
end

local function mock_reads(pr_comments, issue_comments, fields)
  local f = fields or {}
  entity_mocks.mock_pr_view_selector(t, {
    repo = f.repo or repo,
    number = f.pr_number or pr_number,
    state = f.pr_state or "CLOSED",
    head_sha = f.head_sha or head_sha,
    merged_at = f.merged_at,
    comments = pr_comments or {},
  }, entity_mocks.pr_origin_selector)
  entity_mocks.mock_issue_view_selector(t, {
    repo = f.parent_repo or repo,
    number = f.issue_number or issue_number,
    labels = { "fkst-dev:awaiting-pr" },
    comments = issue_comments or {},
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "title,body,comments,labels,state,updatedAt,assignees,author")
  entity_mocks.mock_issue_view_selector(t, {
    repo = f.parent_repo or repo,
    number = f.issue_number or issue_number,
    labels = { "fkst-dev:awaiting-pr" },
    comments = issue_comments or {},
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "title,labels,comments,assignees,author")
end

local function mock_pr_read(pr_comments, fields)
  local f = fields or {}
  entity_mocks.mock_pr_view_selector(t, {
    repo = f.repo or repo,
    number = f.pr_number or pr_number,
    state = f.pr_state or "CLOSED",
    head_sha = f.head_sha or head_sha,
    merged_at = f.merged_at,
    comments = pr_comments or {},
  }, entity_mocks.pr_origin_selector)
end

local function run(payload)
  return t.run_department("departments/on_pr_terminal/main.lua", {
    queue = "devloop_pr_terminal",
    payload = payload or terminal_payload(),
  })
end

return {
  test_pr_terminal_marker_latches_first_trusted_terminal = function()
    local comments = {
      comment(terminal_marker({
        terminal = "merged",
        merge_commit_sha = merge_sha,
        terminal_marker_id = "first-terminal",
      }), core._test_bot_login, "2026-06-03T01:00:00Z"),
      comment(terminal_marker({
        terminal = "closed-unmerged",
        terminal_marker_id = "later-terminal",
      }), core._test_bot_login, "2026-06-03T01:05:00Z"),
    }
    local fact = core.pr_terminal_fact(comments, repo, pr_number, delegation)
    t.eq(fact.terminal, "merged")
    t.eq(fact.terminal_marker_id, "first-terminal")
    t.eq(fact.merge_commit_sha, merge_sha)
    t.eq(core.restart_durable_marker_fields()["pr-terminal"].pr_proposal, true)
    t.eq(core.restart_durable_marker_fields()["child-completed"].idempotency_key, true)
  end,

  test_terminal_classification_excludes_merge_ready = function()
    t.eq(core.classify_pr_terminal_from_view({ state = "closed", merged = true }), "merged")
    t.eq(core.classify_pr_terminal_from_view({ state = "closed", merged = false }), "closed-unmerged")
    t.eq(core.classify_pr_terminal_from_view({ state = "closed", merged_at = "2026-06-03T02:00:00Z" }), "merged")
    t.eq(core.classify_pr_terminal_from_view({ state = "open", merged = false }), nil)
    t.eq(core.classify_pr_terminal_from_view({ state = "merge-ready", merged = false }), nil)
  end,

  test_stranding_race_persists_terminal_before_parent_awaiting_and_resumes_later = function()
    local payload = terminal_payload({ merge_commit_sha = merge_sha })
    mock_env()
    mock_pr_read({})
    local first = run(payload)
    t.eq(first.exit_code, 1)
    local first_write = find_raise(first.raises, "github-proxy.github_pr_comment_request")
    t.is_true(first_write ~= nil)
    t.eq(count_raises(first.raises, "github-proxy.github_pr_comment_request"), 1)
    t.eq(count_raises(first.raises, "github-proxy.github_issue_comment_request"), 0)
    local persisted_comment = comment(first_write.payload.body, core._test_bot_login, "2026-06-03T01:05:00Z")
    local persisted_terminal = core.pr_terminal_fact({ persisted_comment }, repo, pr_number, delegation)
    t.is_true(persisted_terminal ~= nil)
    t.eq(persisted_terminal.terminal, "merged")
    t.eq(persisted_terminal.merge_commit_sha, merge_sha)

    mock_env()
    mock_reads({ persisted_comment }, parent_comments("implementing", { delegation = false }))
    local confirmed_before_parent = run(payload)
    t.eq(confirmed_before_parent.exit_code, 0)
    local second_write = find_raise(confirmed_before_parent.raises, "github-proxy.github_pr_comment_request")
    t.is_true(second_write ~= nil)
    t.eq(count_raises(confirmed_before_parent.raises, "github-proxy.github_pr_comment_request"), 1)
    t.eq(second_write.payload.dedup_key, first_write.payload.dedup_key)
    t.eq(second_write.payload.body, first_write.payload.body)
    t.eq(count_raises(confirmed_before_parent.raises, "github-proxy.github_issue_comment_request"), 0)

    mock_env()
    mock_reads({ persisted_comment }, parent_comments("awaiting-pr"))
    local resumed = run(payload)
    t.eq(resumed.exit_code, 0)
    t.eq(count_raises(resumed.raises, "github-proxy.github_pr_comment_request"), 1)
    local resume = find_raise(resumed.raises, "github-proxy.github_issue_comment_request")
    t.is_true(resume ~= nil)
    t.is_true(resume.payload.body:find('state="merged"', 1, true) ~= nil)
    t.is_true(resume.payload.body:find("fkst:github-devloop:child-completed:v1", 1, true) ~= nil)
    t.is_true(resume.payload.body:find('pr_proposal="' .. child .. '"', 1, true) ~= nil)
  end,

  test_duplicate_terminal_delivery_is_idempotent_on_child_completed_key = function()
    local payload = terminal_payload({ merge_commit_sha = merge_sha })
    local persisted = core.pr_terminal_fact({
      comment(terminal_marker({ merge_commit_sha = merge_sha }), core._test_bot_login),
    }, repo, pr_number, delegation)
    local key = core.pr_terminal_child_completed_key(parent, persisted)
    local completed = core.child_completed_marker({
      proposal_id = parent,
      pr_proposal = child,
      pr_source_ref = "owner/repo#pr/7",
      delegation_generation = delegation,
      terminal_marker_id = "terminal-1",
      terminal = "merged",
      idempotency_key = key,
    })

    mock_env()
    mock_reads({
      comment(terminal_marker({ merge_commit_sha = merge_sha }), core._test_bot_login),
    }, parent_comments("awaiting-pr", { child_completed = completed }))
    local result = run(payload)
    t.eq(result.exit_code, 0)
    t.eq(count_raises(result.raises, "github-proxy.github_pr_comment_request"), 1)
    t.eq(count_raises(result.raises, "github-proxy.github_issue_comment_request"), 0)
  end,

  test_resume_requires_matching_child_proposal_and_generation = function()
    local payload = terminal_payload()
    mock_env()
    mock_reads({
      comment(terminal_marker(), core._test_bot_login),
    }, parent_comments("awaiting-pr", {
      pr_proposal = "github-devloop/pr/owner/repo/8",
      pr_number = 8,
    }))
    local child_mismatch = run(payload)
    t.eq(child_mismatch.exit_code, 0)
    t.eq(count_raises(child_mismatch.raises, "github-proxy.github_issue_comment_request"), 0)

    mock_env()
    mock_reads({
      comment(terminal_marker(), core._test_bot_login),
    }, parent_comments("awaiting-pr", {
      delegation_generation = "delegation-2",
    }))
    local generation_mismatch = run(payload)
    t.eq(generation_mismatch.exit_code, 0)
    t.eq(count_raises(generation_mismatch.raises, "github-proxy.github_issue_comment_request"), 0)
  end,

  test_closed_unmerged_resumes_to_ready_new_generation = function()
    local payload = terminal_payload({ terminal = "closed-unmerged", terminal_marker_id = "closed-terminal" })
    mock_env()
    mock_reads({
      comment(terminal_marker({ terminal = "closed-unmerged", terminal_marker_id = "closed-terminal" }), core._test_bot_login),
    }, parent_comments("awaiting-pr"))
    local result = run(payload)
    t.eq(result.exit_code, 0)
    local resume = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(resume ~= nil)
    t.is_true(resume.payload.body:find('state="ready"', 1, true) ~= nil)
    t.is_true(resume.payload.body:find("/reimplement/1", 1, true) ~= nil)
  end,

  test_behavior_preserving_dark_no_runtime_terminal_emitter = function()
    local root = package_root()
    for _, path in ipairs({
      "departments/observe_pr/main.lua",
      "departments/merge/main.lua",
      "departments/reconcile/main.lua",
      "core/replayer.lua",
    }) do
      local source = read_file(root .. "/" .. path)
      t.eq(source:find('log_raise%([^\\n]-"devloop_pr_terminal"', 1), nil, path)
      t.eq(source:find('raise%([^\\n]-"devloop_pr_terminal"', 1), nil, path)
    end
    for _, row in ipairs(core.restart_transition_table()) do
      if row.from_state ~= "awaiting-pr" then
        t.eq(h.has_value(row.to_states, "awaiting-pr"), false, row.from_state)
      end
    end
    t.eq(h.has_value(core._state_graph.implementing, "awaiting-pr"), false)
  end,
}
