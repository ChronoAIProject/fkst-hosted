local h = require("tests.devloop_helpers")
local bridge = require("contract.external_pr_bridge")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local gh_argv = require("testkit.gh_argv_mock")
local m_builders = require("devloop.markers.builders")
local m_facts = require("devloop.markers.facts")

local t = h.t
local core = h.core
local ready = h.ready
local opts = h.opts
local run_implement = h.run_implement
local mock_issue_implement = h.mock_issue_implement
local mock_fresh_external_pr_implement_worktree = h.mock_fresh_external_pr_implement_worktree
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local mock_git_push = h.mock_git_push
local mock_branch_diff_paths = h.mock_branch_diff_paths
local mock_bot_env = h.mock_bot_env
local mock_write_env = h.mock_write_env
local deterministic_branch_for = h.deterministic_branch_for
local find_raise = h.find_raise
local count_calls = h.count_calls

local external_head = "1234567890abcdef1234567890abcdef12345678"
local implementation_head = "def456"
local base_head = "abc123"

local function pr_list_json(branch)
  return '[{"number":7,"head":{"ref":"' .. branch .. '","sha":"def456"},"base":{"ref":"dev"},"state":"open"}]\n'
end

local function mock_pr_child_created(branch)
  t.mock_command(core.gh_pr_list_head_base_cmd("owner/repo", branch, "dev"), {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh pr create", {
    stdout = "https://github.example/owner/repo/pull/7\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(core.gh_pr_list_head_base_cmd("owner/repo", branch, "dev"), {
    stdout = pr_list_json(branch),
    stderr = "",
    exit_code = 0,
  })
end

local function visible_child_comments(event, branch)
  return {
    m_builders.pr_origin_marker(event.proposal_id, 42, branch, event.dedup_key, "dev")
      .. "\n" .. core.state_marker(event.proposal_id, "pr-open", event.dedup_key),
  }
end

local function visible_issue_comments(event, branch)
  return {
    core.state_marker(event.proposal_id, "implementing", event.dedup_key),
    m_builders.implementing_marker(event.proposal_id, event.dedup_key, branch, implementation_head, "dev", base_head),
    m_builders.pr_delegation_marker(event.proposal_id, "github-devloop/pr/owner/repo/7", 7, event.dedup_key, "g1"),
  }
end

local function mock_linked_pr_view(event, branch)
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = "owner/repo",
    number = 7,
    comments = visible_child_comments(event, branch),
    head = branch,
    head_sha = implementation_head,
    base_branch = "dev",
    state = "OPEN",
  }, entity_read_mocks.pr_origin_selector)
end

local function mock_remote_implementation_branch(branch)
  t.mock_command("git fetch origin " .. tostring(branch), {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("refs/remotes/origin/" .. tostring(branch) .. "^{commit}", {
    stdout = implementation_head .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_real_write_mode()
  for _ = 1, 6 do
    mock_write_env("1")
  end
  mock_bot_env()
end

local function bridge_issue_body()
  return table.concat({
    bridge.marker("owner/repo", 7),
    "",
    "- Source: external PR #7, author @contributor. source_ref: external:owner/repo#pr/7",
    "- Task: the contributor change is already provisioned in your worktree; complete/fix it there.",
  }, "\n")
end

local function mock_bridge_issue(event)
  mock_issue_implement({ "fkst-dev:ready" }, {
    core.state_marker(event.proposal_id, "ready", event.dedup_key),
  }, {
    body = bridge_issue_body(),
    author_login = "fkst-test-bot",
  })
end

local function find_comment_with(raises, text)
  return find_raise(raises, "github-proxy.github_issue_comment_request", function(payload)
    return tostring(payload.body or ""):find(text, 1, true) ~= nil
  end)
end

local function assert_awaiting_pr(result, event)
  local awaiting = find_comment_with(result.raises, 'state="awaiting-pr"')
  t.is_true(awaiting ~= nil)
  t.is_true(awaiting.payload.body:find("fkst:github-devloop:pr-delegation:v1", 1, true) ~= nil)
  local fact = m_facts.pr_delegation_fact({ awaiting.payload.body }, event.proposal_id, event.dedup_key)
  t.eq(fact.pr_number, 7)
  t.eq(fact.version, event.dedup_key)
end

local function assert_no_impl_failed(result)
  t.eq(find_comment_with(result.raises, 'state="impl-failed"'), nil)
end

local function assert_pr_child_handoff(result)
  t.is_true(find_raise(result.raises, "github-proxy.github_pr_comment_request", function(payload)
    return tostring(payload.body or ""):find('state="pr-open"', 1, true) ~= nil
  end) ~= nil)
  t.is_true(find_raise(result.raises, "github-proxy.github_issue_comment_request", function(payload)
    return tostring(payload.body or ""):find("fkst:github-devloop:pr-delegation:v1", 1, true) ~= nil
  end) ~= nil)
end

local function mock_external_bridge_awaiting_redrive(event, branch)
  mock_issue_implement({ "fkst-dev:implementing" }, visible_issue_comments(event, branch), {
    body = bridge_issue_body(),
    author_login = "fkst-test-bot",
  })
  mock_linked_pr_view(event, branch)
  mock_remote_implementation_branch(branch)
  mock_real_write_mode()
end

local function assert_codex_not_asked_to_fetch_external_pr()
  local saw_codex = false
  for _, call in ipairs(t.command_calls()) do
    if tostring(call.rendered or ""):find("codex exec", 1, true) ~= nil then
      saw_codex = true
      local stdin = tostring(call.stdin or "")
      t.eq(stdin:find("refs/pull/7/head", 1, true), nil)
      t.eq(stdin:find("fetch `refs/pull", 1, true), nil)
      t.eq(stdin:find("Re-derive the full diff", 1, true), nil)
    end
  end
  t.eq(saw_codex, true)
end

local function assert_package_side_fetch_before_codex()
  local fetch_index = nil
  local merge_index = nil
  local codex_index = nil
  for index, call in ipairs(t.command_calls()) do
    if gh_argv.call_contains(call, "git fetch 'origin' 'refs/pull/7/head'") then
      fetch_index = fetch_index or index
    elseif gh_argv.call_contains(call, "merge --no-edit '" .. external_head .. "'") then
      merge_index = merge_index or index
    elseif gh_argv.call_contains(call, "codex exec") then
      codex_index = codex_index or index
    end
  end
  t.is_true(fetch_index ~= nil)
  t.is_true(merge_index ~= nil)
  t.is_true(codex_index ~= nil)
  t.is_true(fetch_index < codex_index)
  t.is_true(merge_index < codex_index)
end

return {
  test_external_pr_bridge_provisions_head_and_publishes_clean_branch = function()
    local event = ready()
    local branch = deterministic_branch_for(event)
    mock_bridge_issue(event)
    mock_fresh_external_pr_implement_worktree(nil, {
      pr_number = 7,
      head_sha = external_head,
    })
    mock_implement_codex(0, "External PR was already present.")
    mock_git_status("")
    mock_branch_diff_paths("packages/github-devloop/core.lua\n")
    t.mock_command("rev-list --count", {
      stdout = "1\n",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("rev-parse --verify refs/heads/", {
      stdout = implementation_head .. "\n",
      stderr = "",
      exit_code = 0,
    })
    mock_git_push(branch)
    mock_pr_child_created(branch)
    mock_real_write_mode()

    local result = run_implement(event, opts("implement-external-pr-bridge-clean", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("git fetch 'origin' 'refs/pull/7/head'"), 1)
    t.eq(count_calls("merge --no-edit '" .. external_head .. "'"), 1)
    assert_no_impl_failed(result)
    assert_pr_child_handoff(result)
    assert_package_side_fetch_before_codex()
    assert_codex_not_asked_to_fetch_external_pr()
  end,

  test_external_pr_bridge_dirty_merge_leaves_conflicts_for_codex_resolution = function()
    local event = ready()
    local branch = deterministic_branch_for(event)
    mock_bridge_issue(event)
    mock_fresh_external_pr_implement_worktree(nil, {
      pr_number = 7,
      head_sha = external_head,
      merge_exit_code = 1,
      merge_stderr = "Automatic merge failed; fix conflicts and then commit the result.\n",
      unmerged_stdout = "100644 abc123 1\tpackages/github-devloop/core.lua\n",
    })
    mock_implement_codex(0, "Resolved external PR conflicts locally.")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit(implementation_head, branch)
    mock_git_push(branch)
    mock_pr_child_created(branch)
    mock_real_write_mode()

    local result = run_implement(event, opts("implement-external-pr-bridge-dirty", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("git fetch 'origin' 'refs/pull/7/head'"), 1)
    t.eq(count_calls("merge --no-edit '" .. external_head .. "'"), 1)
    t.eq(count_calls("ls-files -u"), 1)
    assert_no_impl_failed(result)
    assert_pr_child_handoff(result)
    assert_package_side_fetch_before_codex()
    assert_codex_not_asked_to_fetch_external_pr()
  end,

  test_external_pr_bridge_visible_child_redrive_reaches_awaiting_pr = function()
    local event = ready()
    local branch = deterministic_branch_for(event)
    mock_external_bridge_awaiting_redrive(event, branch)

    local result = run_implement(event, opts("implement-external-pr-bridge-awaiting", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    assert_awaiting_pr(result, event)
    assert_no_impl_failed(result)
  end,
}
