local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local h = require("tests.devloop_helpers")
local payloads_builders = require("devloop.payloads.builders")
local m_builders = require("devloop.markers.builders")
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
local find_causal_raise = h.find_causal_raise

local function handoff_state(handoff)
  if handoff.kind == "github-devloop.merge_ready" then
    return "merge-ready"
  end
  if handoff.kind == "github-devloop.fixing" then
    return "fixing"
  end
  return "reviewing"
end

local function run_comment_handoff_from_request(request, comment_id, name)
  t.mock_command("gh api --method GET 'repos/" .. tostring(request.repo) .. "/issues/comments/" .. tostring(comment_id) .. "'", {
    stdout = '{"body":"' .. h.json_string(core.state_marker(request.handoff.proposal_id, handoff_state(request.handoff), request.handoff.version)) .. '","user":{"login":"fkst-test-bot"}}\n',
    stderr = "",
    exit_code = 0,
  })
  return t.run_department("departments/comment_handoff/main.lua", {
    queue = "github-proxy.github_comment_written",
    payload = {
      schema = "github-proxy.comment-written.v1",
      repo = request.repo,
      target = "pr",
      pr_number = request.pr_number,
      comment_id = comment_id,
      request_dedup_key = request.dedup_key,
      dedup_key = tostring(request.dedup_key) .. "/written/" .. tostring(comment_id),
      source_ref = request.source_ref,
      handoff = request.handoff,
    },
  }, opts(name))
end

local function origin_marker(event)
  return m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")
end

local function mock_base_fetch(base_head)
  t.mock_command("git merge-base --is-ancestor", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git fetch origin dev", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git rev-parse --verify 'refs/remotes/origin/dev^{commit}'", {
    stdout = tostring(base_head or "ba5e1234") .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_resolution_delta(exit_code)
  t.mock_command("git merge-tree --write-tree", {
    stdout = "1234abcd\n",
    stderr = exit_code == 0 and "" or "delta is not empty",
    exit_code = 0,
  })
  t.mock_command("git diff --quiet 1234abcd", {
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
    t.eq(find_raise(result.raises, "devloop_merge_ready"), nil)
    local comment_request = find_raise(result.raises, "github-proxy.github_pr_comment_request").payload
    local comment_body = comment_request.body
    t.is_true(comment_body:find("review%-carry%-over:v1") ~= nil)
    t.is_true(comment_body:find('approved_head_sha="' .. event.reviewed_head_sha .. '"', 1, true) ~= nil)
    t.is_true(comment_body:find('new_head_sha="' .. new_head .. '"', 1, true) ~= nil)
    t.is_true(comment_body:find('review_proposal="' .. devloop_base.pr_review_proposal_id("owner/repo", event.pr_number, event.version, new_head) .. '"', 1, true) ~= nil)
    t.is_true(comment_body:find('proof="merge-tree-empty-delta"', 1, true) ~= nil)
    t.eq(comment_request.handoff.kind, "github-devloop.merge_ready")
    t.eq(comment_request.handoff.proposal_id, event.proposal_id)
    t.eq(comment_request.handoff.pr_number, event.pr_number)
    t.eq(comment_request.handoff.version, event.version)
    t.eq(comment_request.handoff.review_proposal_id, devloop_base.pr_review_proposal_id("owner/repo", event.pr_number, event.version, new_head))
    t.eq(comment_request.handoff.review_dedup_key, "consensus:" .. devloop_base.pr_review_proposal_id("owner/repo", event.pr_number, event.version, new_head) .. "/review")
    t.eq(comment_request.handoff.reviewed_head_sha, new_head)
    t.eq(comment_request.handoff.current_head_sha, new_head)
    local handoff = run_comment_handoff_from_request(comment_request, "IC_carry_over_1", "merge-carry-over-comment-handoff")
    t.eq(handoff.exit_code, 0)
    local merge_ready_raise = find_raise(handoff.raises, "devloop_merge_ready").payload
    local expected = payloads_builders.build_devloop_merge_ready_payload(core, event.proposal_id, event.pr_number, event.version, {
      review_proposal_id = devloop_base.pr_review_proposal_id("owner/repo", event.pr_number, event.version, new_head),
      review_dedup_key = "consensus:" .. devloop_base.pr_review_proposal_id("owner/repo", event.pr_number, event.version, new_head) .. "/review",
      reviewed_head_sha = new_head,
      current_head_sha = new_head,
    }, entity_lib.pr_source_ref("owner/repo", event.pr_number))
    t.eq(merge_ready_raise.schema, expected.schema)
    t.eq(merge_ready_raise.proposal_id, expected.proposal_id)
    t.eq(merge_ready_raise.pr_number, expected.pr_number)
    t.eq(merge_ready_raise.version, expected.version)
    t.eq(merge_ready_raise.review_proposal_id, expected.review_proposal_id)
    t.eq(merge_ready_raise.review_dedup_key, expected.review_dedup_key)
    t.eq(merge_ready_raise.reviewed_head_sha, expected.reviewed_head_sha)
    t.eq(merge_ready_raise.dedup_key, expected.dedup_key)
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
    t.eq(find_causal_raise(result, "devloop_reviewing").payload.version, core.next_review_loop_version(event.version))
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
    t.eq(find_causal_raise(result, "devloop_reviewing").payload.version, core.next_review_loop_version(event.version))
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
    t.eq(find_raise(carry_result.raises, "devloop_merge_ready"), nil)
    local carry_request = find_raise(carry_result.raises, "github-proxy.github_pr_comment_request").payload
    local handoff_result = run_comment_handoff_from_request(carry_request, "IC_carry_over_ci_red_1", "merge-carry-over-ci-red-comment-handoff")
    t.eq(handoff_result.exit_code, 0)
    local carried = find_raise(handoff_result.raises, "devloop_merge_ready", function(payload)
      return payload.reviewed_head_sha == new_head
    end)
    t.eq(carried.payload.reviewed_head_sha, new_head)

    local carried_comments = merge_comments(event)
    local carry_body = carry_request.body
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
    t.eq(find_causal_raise(ci_red, "devloop_fixing").payload.reviewed_head_sha, new_head)
    t.eq(count_calls("gh pr merge"), 0)
  end,
}
