local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local merge_ready = h.merge_ready
local run_merge = h.run_merge
local mock_bot_env = h.mock_bot_env
local mock_write_env = h.mock_write_env
local mock_issue_merge = h.mock_issue_merge
local mock_pr_merge = h.mock_pr_merge
local merge_comments = h.merge_comments
local count_calls = h.count_calls
local find_raise = h.find_raise

local function origin_marker(event)
  return core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")
end

local function mock_base_fetch(base_head)
  t.mock_command("git merge-base --is-ancestor", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git fetch 'origin' 'dev'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("refs/remotes/'origin'/'dev'^{commit}", {
    stdout = tostring(base_head or "ba5e1234") .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_resolution_delta(exit_code)
  t.mock_command("git merge-tree --write-tree", {
    stdout = "",
    stderr = exit_code == 0 and "" or "delta is not empty",
    exit_code = exit_code,
  })
end

return {
  test_merge_carries_over_conflict_only_resolution_head_without_reviewing = function()
    local event = merge_ready()
    local new_head = "feedface"
    mock_bot_env()
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge({ origin_marker(event) }, "devloop-owner-repo-42-01HY", new_head)
    mock_base_fetch("ba5e1234")
    mock_resolution_delta(0)

    local result = run_merge(event, opts("merge-carry-over-resolution-head", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 0)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
    local merge_raise = find_raise(result.raises, "devloop_merge_ready", function(payload)
      return payload.reviewed_head_sha == new_head
    end)
    t.eq(merge_raise.payload.reviewed_head_sha, new_head)
    local comment_body = find_raise(result.raises, "github-proxy.github_pr_comment_request").payload.body
    t.is_true(comment_body:find("review%-carry%-over:v1") ~= nil)
    t.is_true(comment_body:find('approved_head_sha="' .. event.reviewed_head_sha .. '"', 1, true) ~= nil)
    t.is_true(comment_body:find('new_head_sha="' .. new_head .. '"', 1, true) ~= nil)
    t.is_true(comment_body:find('review_proposal="' .. core.pr_review_proposal_id("owner/repo", event.pr_number, event.version, new_head) .. '"', 1, true) ~= nil)
    t.is_true(comment_body:find('proof="merge-tree-empty-delta"', 1, true) ~= nil)
    t.eq(count_calls("git merge-base --is-ancestor"), 1)
    t.eq(count_calls("git merge-tree --write-tree"), 1)
  end,

  test_merge_force_pushed_head_requires_review = function()
    local event = merge_ready()
    local new_head = "feedface"
    mock_bot_env()
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge({ origin_marker(event) }, "devloop-owner-repo-42-01HY", new_head)
    t.mock_command("git merge-base --is-ancestor", {
      stdout = "",
      stderr = "not ancestor",
      exit_code = 1,
    })

    local result = run_merge(event, opts("merge-carry-over-force-push", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_merge_ready"), nil)
    t.eq(find_raise(result.raises, "devloop_reviewing").payload.version, core.next_review_loop_version(event.version))
    t.eq(count_calls("git merge-tree --write-tree"), 0)
    t.eq(count_calls("gh pr merge"), 0)
  end,

  test_merge_non_empty_resolution_delta_requires_review = function()
    local event = merge_ready()
    local new_head = "feedface"
    mock_bot_env()
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge({ origin_marker(event) }, "devloop-owner-repo-42-01HY", new_head)
    mock_base_fetch("ba5e1234")
    mock_resolution_delta(1)

    local result = run_merge(event, opts("merge-carry-over-non-empty-delta", { FKST_GITHUB_WRITE = "1" }))

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_merge_ready"), nil)
    t.eq(find_raise(result.raises, "devloop_reviewing").payload.version, core.next_review_loop_version(event.version))
    t.eq(count_calls("git merge-tree --write-tree"), 1)
    t.eq(count_calls("gh pr merge"), 0)
  end,

  test_merge_carried_approval_ci_red_stays_in_fixing_without_rereview = function()
    local event = merge_ready()
    local new_head = "feedface"
    mock_bot_env()
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge({ origin_marker(event) }, "devloop-owner-repo-42-01HY", new_head)
    mock_base_fetch("ba5e1234")
    mock_resolution_delta(0)

    local carry_result = run_merge(event, opts("merge-carry-over-before-ci-red", { FKST_GITHUB_WRITE = "1" }))

    t.eq(carry_result.exit_code, 0)
    local carried = find_raise(carry_result.raises, "devloop_merge_ready", function(payload)
      return payload.reviewed_head_sha == new_head
    end)
    t.eq(carried.payload.reviewed_head_sha, new_head)

    local carried_comments = merge_comments(event)
    local carry_body = find_raise(carry_result.raises, "github-proxy.github_pr_comment_request").payload.body
    table.insert(carried_comments, carry_body)

    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, carried_comments)
    mock_pr_merge(carried_comments, "devloop-owner-repo-42-01HY", new_head, "OPEN", "owner/repo", false, "MERGEABLE", "CLEAN", "COMPLETED", "FAILURE")
    h.mock_required_check_runs_for(new_head, "failure")

    local ci_red = run_merge(carried.payload, opts("merge-carry-over-ci-red", { FKST_GITHUB_WRITE = "1" }))

    t.eq(ci_red.exit_code, 0)
    t.eq(find_raise(ci_red.raises, "devloop_reviewing"), nil)
    t.eq(find_raise(ci_red.raises, "devloop_merge_ready"), nil)
    t.eq(find_raise(ci_red.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:fixing")
    t.eq(find_raise(ci_red.raises, "devloop_fixing").payload.reviewed_head_sha, new_head)
    t.eq(count_calls("gh pr merge"), 0)
  end,
}
