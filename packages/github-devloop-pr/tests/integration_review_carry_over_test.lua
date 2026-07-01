local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local h = require("tests.devloop_helpers")
local payloads_builders = require("devloop.payloads.builders")
local m_builders = require("devloop.markers.builders")
local t = h.t
local core = h.core
local opts = h.opts
local mock_pr_origin = h.mock_pr_origin
local mock_issue_reviewing = h.mock_issue_reviewing
local merge_comments = h.merge_comments
local mock_pr_normal_risk_diff_name_only = h.mock_pr_normal_risk_diff_name_only
local run_observe_pr = h.run_observe_pr
local find_raise = h.find_raise
local find_causal_raise = h.find_causal_raise
local count_calls = h.count_calls

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
  test_observe_pr_carries_over_approved_head_for_empty_resolution_delta = function()
    local event = h.merge_ready()
    local old_head = event.reviewed_head_sha
    local new_head = "feedface"
    local base_head = "ba5e1234"
    mock_pr_origin({
      m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
    }, "devloop-owner-repo-42-01HY", new_head)
    mock_issue_reviewing({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_base_fetch(base_head)
    mock_resolution_delta(0)
    mock_pr_normal_risk_diff_name_only()

    local result = run_observe_pr(pr_event(), opts("review-carry-over-empty-delta"))

    t.eq(result.exit_code, 0)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.eq(find_raise(result.raises, "devloop_merge_ready"), nil)
    t.is_true(comment_raise.payload.body:find("review%-carry%-over:v1") ~= nil)
    t.is_true(comment_raise.payload.body:find('approved_head_sha="' .. old_head .. '"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('new_head_sha="' .. new_head .. '"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('base_head_sha="' .. base_head .. '"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('proof="merge-tree-empty-delta"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('decision="approve"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('head_sha="' .. new_head .. '"', 1, true) ~= nil)
    t.eq(comment_raise.payload.handoff.kind, "github-devloop.merge_ready")
    t.eq(comment_raise.payload.handoff.proposal_id, event.proposal_id)
    t.eq(comment_raise.payload.handoff.pr_number, event.pr_number)
    t.eq(comment_raise.payload.handoff.version, event.version)
    t.eq(comment_raise.payload.handoff.review_proposal_id, devloop_base.pr_review_proposal_id("owner/repo", 7, event.version, new_head))
    t.eq(comment_raise.payload.handoff.review_dedup_key, "consensus:" .. devloop_base.pr_review_proposal_id("owner/repo", 7, event.version, new_head) .. "/review")
    t.eq(comment_raise.payload.handoff.reviewed_head_sha, new_head)
    t.eq(comment_raise.payload.handoff.current_head_sha, new_head)
    t.is_true(comment_raise.payload.body:find('review_proposal="' .. devloop_base.pr_review_proposal_id("owner/repo", 7, event.version, new_head) .. '"', 1, true) ~= nil)
    local handoff = run_comment_handoff_from_request(comment_raise.payload, "IC_replay_carry_over_1", "review-carry-over-replayer-comment-handoff")
    t.eq(handoff.exit_code, 0)
    local merge_raise = find_raise(handoff.raises, "devloop_merge_ready", function(payload)
      return payload.reviewed_head_sha == new_head
    end)
    local expected = payloads_builders.build_devloop_merge_ready_payload(core, event.proposal_id, event.pr_number, event.version, {
      review_proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, event.version, new_head),
      review_dedup_key = "consensus:" .. devloop_base.pr_review_proposal_id("owner/repo", 7, event.version, new_head) .. "/review",
      reviewed_head_sha = new_head,
      current_head_sha = new_head,
    }, entity_lib.pr_source_ref("owner/repo", event.pr_number))
    t.eq(merge_raise.payload.schema, expected.schema)
    t.eq(merge_raise.payload.proposal_id, expected.proposal_id)
    t.eq(merge_raise.payload.pr_number, expected.pr_number)
    t.eq(merge_raise.payload.version, expected.version)
    t.eq(merge_raise.payload.review_proposal_id, expected.review_proposal_id)
    t.eq(merge_raise.payload.review_dedup_key, expected.review_dedup_key)
    t.eq(merge_raise.payload.reviewed_head_sha, expected.reviewed_head_sha)
    t.eq(merge_raise.payload.dedup_key, expected.dedup_key)
    t.is_true(merge_raise.payload.dedup_key ~= event.dedup_key)
    t.is_true(merge_raise.payload.dedup_key:find(new_head, 1, true) ~= nil)
    t.eq(count_calls("git merge-tree --write-tree"), 1)
  end,

  test_observe_pr_merge_ready_replay_dedup_tracks_current_head = function()
    local event = h.merge_ready()
    local old_head = event.reviewed_head_sha
    local advanced_head = "feedface"
    mock_pr_origin({
      m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
    }, "devloop-owner-repo-42-01HY", old_head)
    mock_issue_reviewing({ "fkst-dev:merge-ready" }, merge_comments(event))

    local unchanged = run_observe_pr(pr_event("2026-06-04T01:02:03Z"), opts("review-carry-over-dedup-unchanged-head"))
    t.eq(unchanged.exit_code, 0)
    local unchanged_merge = find_raise(unchanged.raises, "devloop_merge_ready")
    t.eq(unchanged_merge.payload.dedup_key, event.dedup_key)

    mock_pr_origin({
      m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
    }, "devloop-owner-repo-42-01HY", advanced_head)
    mock_issue_reviewing({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_base_fetch("ba5e1234")
    mock_resolution_delta(1)

    local advanced = run_observe_pr(pr_event("2026-06-04T01:02:04Z"), opts("review-carry-over-dedup-head-advanced"))
    t.eq(advanced.exit_code, 0)
    local replayed_merge = find_raise(advanced.raises, "devloop_merge_ready")
    t.is_true(replayed_merge ~= nil)
    t.eq(find_raise(advanced.raises, "devloop_reviewing"), nil)
    local replay_payload = payloads_builders.build_devloop_merge_ready_payload(core, event.proposal_id, event.pr_number, event.version, {
      review_proposal_id = event.review_proposal_id,
      review_dedup_key = event.review_dedup_key,
      reviewed_head_sha = old_head,
      current_head_sha = advanced_head,
    }, event.source_ref)
    t.is_true(replay_payload.dedup_key ~= event.dedup_key)
    t.is_true(replay_payload.dedup_key:find(advanced_head, 1, true) ~= nil)
  end,

  test_observe_pr_non_empty_resolution_delta_falls_back_to_full_review = function()
    local event = h.merge_ready()
    local new_head = "feedface"
    mock_pr_origin({
      m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
    }, "devloop-owner-repo-42-01HY", new_head)
    mock_issue_reviewing({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_base_fetch("ba5e1234")
    mock_resolution_delta(1)

    local result = run_observe_pr(pr_event("2026-06-04T01:02:04Z"), opts("review-carry-over-non-empty-delta"))

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_merge_ready"), nil)
    local reviewing_raise = find_causal_raise(result, "devloop_reviewing")
    t.eq(reviewing_raise.payload.version, event.version)
    t.eq(count_calls("git merge-tree --write-tree"), 1)
  end,

  test_observe_pr_carry_over_is_idempotent_when_new_review_result_visible = function()
    local event = h.merge_ready()
    local new_head = "feedface"
    local new_review = devloop_base.pr_review_proposal_id("owner/repo", 7, event.version, new_head)
    local comments = merge_comments(event)
    table.insert(comments, m_builders.review_result_marker(core, new_review, event.proposal_id, "approve", "consensus:" .. new_review .. "/review"))
    table.insert(comments, m_builders.merge_ready_marker(core, event.proposal_id, event.pr_number, event.version, new_review, "consensus:" .. new_review .. "/review", new_head))
    mock_pr_origin({
      m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
    }, "devloop-owner-repo-42-01HY", new_head)
    mock_issue_reviewing({ "fkst-dev:merge-ready" }, comments)
    mock_base_fetch("ba5e1234")
    mock_resolution_delta(0)

    local result = run_observe_pr(pr_event("2026-06-04T01:02:05Z"), opts("review-carry-over-idempotent"))

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request"), nil)
    local merge_raise = find_raise(result.raises, "devloop_merge_ready", function(payload)
      return payload.reviewed_head_sha == new_head
    end)
    t.eq(merge_raise.payload.reviewed_head_sha, new_head)
    t.eq(merge_raise.payload.review_proposal_id, new_review)
  end,
}
