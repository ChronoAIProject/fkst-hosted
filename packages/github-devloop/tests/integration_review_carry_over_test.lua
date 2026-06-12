local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local mock_pr_origin = h.mock_pr_origin
local mock_issue_reviewing = h.mock_issue_reviewing
local merge_comments = h.merge_comments
local run_observe_pr = h.run_observe_pr
local find_raise = h.find_raise
local count_calls = h.count_calls

local function pr_event(updated_at)
  return {
    schema = "github-proxy.v1",
    type = "pr",
    repo = "owner/repo",
    number = 7,
    dedup_key = "owner/repo#pr#7@" .. tostring(updated_at or "2026-06-04T01:02:03Z"),
    source_ref = {
      kind = "external",
      ref = "owner/repo#pr/7",
    },
  }
end

local function mock_base_fetch(base_head)
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
  test_observe_pr_carries_over_approved_head_for_empty_resolution_delta = function()
    local event = h.merge_ready()
    local old_head = event.reviewed_head_sha
    local new_head = "feedface"
    local base_head = "ba5e1234"
    mock_pr_origin({
      core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
    }, "devloop-owner-repo-42-01HY", new_head)
    mock_issue_reviewing({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_base_fetch(base_head)
    mock_resolution_delta(0)

    local result = run_observe_pr(pr_event(), opts("review-carry-over-empty-delta"))

    t.eq(result.exit_code, 0)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local merge_raise = find_raise(result.raises, "devloop_merge_ready")
    t.is_true(comment_raise.payload.body:find("review%-carry%-over:v1") ~= nil)
    t.is_true(comment_raise.payload.body:find('approved_head_sha="' .. old_head .. '"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('new_head_sha="' .. new_head .. '"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('base_head_sha="' .. base_head .. '"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('proof="merge-tree-empty-delta"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('decision="approve"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('head_sha="' .. new_head .. '"', 1, true) ~= nil)
    t.eq(merge_raise.payload.reviewed_head_sha, new_head)
    t.eq(merge_raise.payload.review_proposal_id, core.pr_review_proposal_id("owner/repo", 7, event.version, new_head))
    t.eq(count_calls("git merge-tree --write-tree"), 1)
  end,

  test_observe_pr_non_empty_resolution_delta_falls_back_to_full_review = function()
    local event = h.merge_ready()
    local new_head = "feedface"
    mock_pr_origin({
      core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
    }, "devloop-owner-repo-42-01HY", new_head)
    mock_issue_reviewing({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_base_fetch("ba5e1234")
    mock_resolution_delta(1)

    local result = run_observe_pr(pr_event("2026-06-04T01:02:04Z"), opts("review-carry-over-non-empty-delta"))

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_merge_ready"), nil)
    local reviewing_raise = find_raise(result.raises, "devloop_reviewing")
    t.eq(reviewing_raise.payload.version, event.version)
    t.eq(count_calls("git merge-tree --write-tree"), 1)
  end,

  test_observe_pr_carry_over_is_idempotent_when_new_review_result_visible = function()
    local event = h.merge_ready()
    local new_head = "feedface"
    local new_review = core.pr_review_proposal_id("owner/repo", 7, event.version, new_head)
    local comments = merge_comments(event)
    table.insert(comments, core.review_result_marker(new_review, event.proposal_id, "approve", "consensus:" .. new_review .. "/review"))
    table.insert(comments, core.merge_ready_marker(event.proposal_id, event.pr_number, event.version, new_review, "consensus:" .. new_review .. "/review", new_head))
    mock_pr_origin({
      core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
    }, "devloop-owner-repo-42-01HY", new_head)
    mock_issue_reviewing({ "fkst-dev:merge-ready" }, comments)
    mock_base_fetch("ba5e1234")
    mock_resolution_delta(0)

    local result = run_observe_pr(pr_event("2026-06-04T01:02:05Z"), opts("review-carry-over-idempotent"))

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request"), nil)
    local merge_raise = find_raise(result.raises, "devloop_merge_ready")
    t.eq(merge_raise.payload.reviewed_head_sha, new_head)
    t.eq(merge_raise.payload.review_proposal_id, new_review)
  end,
}
