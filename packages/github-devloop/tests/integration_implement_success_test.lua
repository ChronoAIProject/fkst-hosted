local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local ready = h.ready
local run_implement = h.run_implement
local mock_issue_implement = h.mock_issue_implement
local deterministic_branch_for = h.deterministic_branch_for
local mock_fresh_implement_worktree = h.mock_fresh_implement_worktree
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local count_calls = h.count_calls
local find_raise = h.find_raise

local current_base_pin = "2222222222222222222222222222222222222222"
local stale_queue_pin = "1111111111111111111111111111111111111111"

local function find_comment_with(raises, text)
  return find_raise(raises, "github-proxy.github_issue_comment_request", function(payload)
    return tostring(payload.body or ""):find(text, 1, true) ~= nil
  end)
end

return {
  test_implement_ready_runs_codex_in_worktree_and_marks_implementing = function()
    local event = ready()
    local branch = deterministic_branch_for(event)
    mock_issue_implement({ "fkst-dev:ready", "fkst-dev:thinking" })
    mock_fresh_implement_worktree()
    mock_implement_codex(0, "implemented")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("def456", branch)
    local result = run_implement(event, opts("implement-success"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 4)
    local attempt_raise = find_comment_with(result.raises, "fkst:github-devloop:implement-attempt:v1")
    t.is_true(attempt_raise.payload.body:find('proposal="' .. event.proposal_id .. '"', 1, true) ~= nil)
    t.is_true(attempt_raise.payload.body:find('dedup="' .. event.dedup_key .. '"', 1, true) ~= nil)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    local state_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request", function(payload)
      return tostring(payload.body or ""):find("github-devloop implementation worktree ready", 1, true) ~= nil
    end)
    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request", function(payload)
      return tostring(payload.body or ""):find("github-devloop implementation output published", 1, true) ~= nil
    end)
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:implementing")
    t.eq(#label_raise.payload.remove_labels, 12)
    t.eq(attempt_raise.payload.body, state_raise.payload.body)
    t.is_true(state_raise.payload.body:find(core.state_marker(event.proposal_id, "implementing", event.dedup_key), 1, true) ~= nil)
    t.is_true(state_raise.payload.body:find("fkst:github-devloop:implement-attempt:v1", 1, true) ~= nil)
    t.eq(core.implementing_fact({ state_raise.payload.body }, event.proposal_id, event.dedup_key), nil)
    t.is_true(comment_raise.payload.body:find("github-devloop implementation output published", 1, true) ~= nil)
    t.eq(comment_raise.payload.body:find(core.state_marker(event.proposal_id, "implementing", event.dedup_key), 1, true), nil)
    local outcome_attempt_raise = find_comment_with(result.raises, "github-devloop implementation attempt started")
    t.is_true(outcome_attempt_raise ~= nil)
    t.eq(outcome_attempt_raise.payload.body:find(core.state_marker(event.proposal_id, "implementing", event.dedup_key), 1, true), nil)
    local fact = core.implementing_fact({ comment_raise.payload.body }, event.proposal_id, event.dedup_key)
    t.eq(fact.branch, branch)
    t.eq(fact.head_sha, "def456")
    local calls = t.command_calls()
    local saw_worktree_prefix = false
    local saw_prompt = false
    for _, call in ipairs(calls) do
      if call.rendered:find("codex exec", 1, true) ~= nil then
        saw_worktree_prefix = call.rendered:find("devloop-owner-repo-42", 1, true) ~= nil
        saw_prompt = call.stdin:find("Do not open a pull request.", 1, true) ~= nil
      end
    end
    t.eq(saw_worktree_prefix, true)
    t.eq(saw_prompt, true)
    t.eq(count_calls("git -C"), 10)
    t.eq(count_calls("git worktree add -b"), 1)
    t.eq(count_calls("codex exec"), 1)
    t.eq(count_calls("status --porcelain"), 1)
    t.eq(count_calls("add -A"), 2)
    t.eq(count_calls("commit -m"), 2)
  end,

  test_implement_refreshes_substrate_ref_to_current_base_before_codex = function()
    local event = ready()
    local branch = deterministic_branch_for(event)
    mock_issue_implement({ "fkst-dev:ready", "fkst-dev:thinking" })
    local worktree = mock_fresh_implement_worktree(nil, current_base_pin, stale_queue_pin)
    mock_implement_codex(0, "implemented after current substrate pin")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("def456", branch)

    local result = run_implement(event, opts("implement-refreshes-substrate-pin"))
    t.eq(result.exit_code, 0)
    t.eq(file.read(worktree .. "/.fkst/substrate-ref"), current_base_pin .. "\n")

    local base_pin_read_index = nil
    local branch_pin_read_index = nil
    local pin_commit_index = nil
    local codex_index = nil
    for index, call in ipairs(t.command_calls()) do
      if call.rendered:find("git show abc123:.fkst/substrate-ref", 1, true) ~= nil then
        base_pin_read_index = base_pin_read_index or index
      elseif call.rendered:find("git show", 1, true) ~= nil
        and call.rendered:find(":.fkst/substrate-ref", 1, true) ~= nil then
        branch_pin_read_index = branch_pin_read_index or index
      elseif call.rendered:find("commit -m 'chore: refresh fkst-substrate pin'", 1, true) ~= nil then
        pin_commit_index = index
      elseif call.rendered:find("codex exec", 1, true) ~= nil then
        codex_index = index
      end
    end
    t.is_true(base_pin_read_index ~= nil)
    t.is_true(branch_pin_read_index ~= nil)
    t.is_true(pin_commit_index ~= nil)
    t.is_true(codex_index ~= nil)
    t.is_true(base_pin_read_index < codex_index)
    t.is_true(branch_pin_read_index < codex_index)
    t.is_true(pin_commit_index < codex_index)
  end,
}
