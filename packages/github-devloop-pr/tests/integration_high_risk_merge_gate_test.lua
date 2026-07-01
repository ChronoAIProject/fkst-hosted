local entity_lib = require("devloop.entity")
local h = require("tests.devloop_helpers")
local m_facts = require("devloop.markers.facts")
local t = h.t
local core = h.core
local opts = h.opts
local merge_ready = h.merge_ready
local run_merge = h.run_merge
local run_observe_pr = h.run_observe_pr
local mock_bot_env = h.mock_bot_env
local mock_write_env = h.mock_write_env
local mock_issue_merge = h.mock_issue_merge
local mock_issue_reviewing = h.mock_issue_reviewing
local mock_pr_merge = h.mock_pr_merge
local mock_pr_origin = h.mock_pr_origin
local mock_merging_comment = h.mock_merging_comment
local mock_pr_merge_command = h.mock_pr_merge_command
local mock_pr_high_risk_diff_name_only = h.mock_pr_high_risk_diff_name_only
local mock_pr_normal_risk_diff_name_only = h.mock_pr_normal_risk_diff_name_only
local mock_pr_empty_diff_name_only = h.mock_pr_empty_diff_name_only
local mock_pr_failed_diff_name_only = h.mock_pr_failed_diff_name_only
local merge_comments = h.merge_comments
local merge_comments_with_high_risk_evidence = h.merge_comments_with_high_risk_evidence
local high_risk_review_evidence_marker = h.high_risk_review_evidence_marker
local count_calls = h.count_calls
local find_raise = h.find_raise
local find_causal_raise = h.find_causal_raise
local high_risk_merge_gate = require("core.high_risk_merge_gate")
local m_builders = require("devloop.markers.builders")

local function origin_marker(event)
  return m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")
end

local function comments_with_untrusted_evidence(event)
  local comments = merge_comments(event)
  table.insert(comments, {
    body = high_risk_review_evidence_marker(event),
    author_login = "mallory",
  })
  return comments
end

local function comments_with_mismatched_evidence(event, fields)
  local comments = merge_comments(event)
  table.insert(comments, high_risk_review_evidence_marker(event, fields))
  return comments
end

local function comments_with_older_matching_evidence_before_newer(event)
  local comments = merge_comments(event)
  table.insert(comments, {
    body = high_risk_review_evidence_marker(event, { angle_digest = "older-angle-digest" }),
    author_login = "fkst-test-bot",
    created_at = "2026-06-03T01:00:00Z",
  })
  table.insert(comments, {
    body = high_risk_review_evidence_marker(event, { angle_digest = "newer-angle-digest" }),
    author_login = "fkst-test-bot",
    created_at = "2026-06-03T01:01:00Z",
  })
  return comments
end

local function mock_successful_merge_path(event, comments)
  mock_issue_merge({ "fkst-dev:merge-ready" }, comments)
  mock_pr_merge({ origin_marker(event) })
  mock_issue_merge({ "fkst-dev:merge-ready" }, comments)
  mock_pr_merge(comments)
  mock_merging_comment()
  mock_pr_merge_command()
  mock_pr_merge(comments, "devloop-owner-repo-42-01HY", event.reviewed_head_sha, "MERGED", "owner/repo", false, "MERGEABLE", "CLEAN", "COMPLETED", "SUCCESS", "2026-06-03T02:03:04Z")
end

local function event_comments(event, comments)
  if type(comments) == "function" then
    return comments(event)
  end
  return comments or merge_comments(event)
end

local function run_write_merge_with_comments(name, comments, risk_mock)
  local event = merge_ready()
  mock_bot_env()
  mock_write_env("1")
  mock_write_env("1")
  risk_mock()
  risk_mock()
  mock_successful_merge_path(event, event_comments(event, comments))
  return event, run_merge(event, opts(name, { FKST_GITHUB_WRITE = "1", FKST_TEST_SKIP_DEFAULT_RISK_MOCK = "1" }))
end

local function assert_high_risk_without_merge(name, comments, risk_mock)
  local event = merge_ready()
  mock_bot_env()
  mock_write_env("1")
  mock_write_env("1")
  risk_mock()
  risk_mock()
  mock_successful_merge_path(event, event_comments(event, comments))

  local result = run_merge(event, opts(name, { FKST_GITHUB_WRITE = "1", FKST_TEST_SKIP_DEFAULT_RISK_MOCK = "1" }))
  t.eq(result.exit_code, 1)
  t.eq(#result.raises, 0)
  t.eq(count_calls("gh pr merge"), 0)
end

return {
  test_unknown_diff_name_risk_never_accepts_evidence = function()
    local evidence_checked = false
    local fake_core = {
      gh_pr_diff_name_only = function()
        return { stdout = "", stderr = "diff unavailable", exit_code = 1 }
      end,
      high_risk_review_evidence_fact = function()
        evidence_checked = true
        return { verdict = "approve" }
      end,
    }
    local ok, reason = high_risk_merge_gate.require_evidence(fake_core, "owner/repo", {}, merge_ready())
    t.eq(ok, false)
    t.eq(evidence_checked, false)
    t.is_true(tostring(reason):find("retry%-pending%(high%-risk%-review%-evidence:diff%-name%-only%-failed%)") ~= nil)
  end,

  test_normal_risk_merge_path_stays_unchanged = function()
    local event, result = run_write_merge_with_comments(
      "merge-normal-risk-unchanged",
      nil,
      mock_pr_normal_risk_diff_name_only
    )
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(count_calls("gh pr merge"), 1)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request").payload.pr_number, event.pr_number)
  end,

  test_high_risk_without_evidence_retries_without_merging_direct_path = function()
    assert_high_risk_without_merge("merge-high-risk-no-evidence", nil, mock_pr_high_risk_diff_name_only)
  end,

  test_high_risk_with_matching_trusted_evidence_merges = function()
    local _, result = run_write_merge_with_comments(
      "merge-high-risk-with-evidence",
      merge_comments_with_high_risk_evidence,
      mock_pr_high_risk_diff_name_only
    )
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 1)
  end,

  test_high_risk_untrusted_evidence_is_ignored = function()
    assert_high_risk_without_merge(
      "merge-high-risk-untrusted-evidence",
      comments_with_untrusted_evidence,
      mock_pr_high_risk_diff_name_only
    )
  end,

  test_high_risk_wrong_head_evidence_is_ignored = function()
    assert_high_risk_without_merge(
      "merge-high-risk-wrong-head-evidence",
      function(event)
        return comments_with_mismatched_evidence(event, { head_sha = "feedface" })
      end,
      mock_pr_high_risk_diff_name_only
    )
  end,

  test_high_risk_wrong_digest_evidence_is_ignored = function()
    assert_high_risk_without_merge(
      "merge-high-risk-wrong-digest-evidence",
      function(event)
        return comments_with_mismatched_evidence(event, { paths_digest = "wrong-digest" })
      end,
      mock_pr_high_risk_diff_name_only
    )
  end,

  test_high_risk_evidence_lookup_requires_paths_digest = function()
    local event = merge_ready()
    local fact = m_facts.high_risk_review_evidence_fact(core, 
      merge_comments_with_high_risk_evidence(event),
      event.proposal_id,
      event.version,
      event.pr_number,
      event.reviewed_head_sha,
      event.review_proposal_id,
      event.review_dedup_key,
      nil
    )
    t.eq(fact, nil)
  end,

  test_high_risk_evidence_lookup_uses_newest_valid_matching_digest = function()
    local event = merge_ready()
    local fact = m_facts.high_risk_review_evidence_fact(core, 
      comments_with_older_matching_evidence_before_newer(event),
      event.proposal_id,
      event.version,
      event.pr_number,
      event.reviewed_head_sha,
      event.review_proposal_id,
      event.review_dedup_key,
      h.high_risk_paths_digest()
    )
    t.is_true(fact ~= nil)
    t.eq(fact.paths_digest, h.high_risk_paths_digest())
    t.eq(fact.comment_created_at, "2026-06-03T01:01:00Z")
    t.eq(fact.angle_digest, "newer-angle-digest")
  end,

  test_diff_name_fetch_failure_treats_as_high_risk = function()
    assert_high_risk_without_merge("merge-risk-fetch-failure", nil, mock_pr_failed_diff_name_only)
  end,

  test_empty_diff_name_treats_as_high_risk = function()
    assert_high_risk_without_merge("merge-risk-empty-paths", nil, mock_pr_empty_diff_name_only)
  end,

  test_diff_name_fetch_failure_rejects_otherwise_matching_evidence = function()
    assert_high_risk_without_merge(
      "merge-risk-fetch-failure-with-evidence",
      merge_comments_with_high_risk_evidence,
      mock_pr_failed_diff_name_only
    )
  end,

  test_empty_diff_name_rejects_otherwise_matching_evidence = function()
    assert_high_risk_without_merge(
      "merge-risk-empty-paths-with-evidence",
      merge_comments_with_high_risk_evidence,
      mock_pr_empty_diff_name_only
    )
  end,

  test_replayer_raised_high_risk_merge_ready_still_needs_evidence = function()
    local event = merge_ready()
    mock_bot_env()
    mock_pr_origin({ origin_marker(event) })
    mock_issue_reviewing({ "fkst-dev:merge-ready" }, merge_comments(event))

    local observed = run_observe_pr({
      schema = "github-proxy.v1",
      repo = "owner/repo",
      type = "pr",
      number = event.pr_number,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
      source_ref = entity_lib.pr_source_ref("owner/repo", event.pr_number),
    }, opts("observe-pr-replays-high-risk-merge-ready"))
    local replayed = find_raise(observed.raises, "devloop_merge_ready")
    t.is_true(replayed ~= nil)

    mock_write_env("1")
    mock_pr_high_risk_diff_name_only()
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge(merge_comments(event))

    local result = run_merge(replayed.payload, opts("merge-replayed-high-risk-no-evidence", { FKST_GITHUB_WRITE = "1", FKST_TEST_SKIP_DEFAULT_RISK_MOCK = "1" }))
    t.eq(result.exit_code, 1)
    t.eq(count_calls("gh pr merge"), 0)
  end,

  test_carry_over_high_risk_merge_ready_still_needs_evidence = function()
    local event = merge_ready()
    local new_head = "feedface"
    mock_bot_env()
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge({ origin_marker(event) }, "devloop-owner-repo-42-01HY", new_head)
    t.mock_command("git merge-base --is-ancestor", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("git fetch origin dev", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("git rev-parse --verify 'refs/remotes/origin/dev^{commit}'", { stdout = "ba5e1234\n", stderr = "", exit_code = 0 })
    t.mock_command("git merge-tree --write-tree", { stdout = "1234abcd\n", stderr = "", exit_code = 0 })
    t.mock_command("git diff --quiet 1234abcd", { stdout = "", stderr = "", exit_code = 0 })
    mock_pr_high_risk_diff_name_only()

    local carry = run_merge(event, opts("merge-carry-over-high-risk-source", { FKST_GITHUB_WRITE = "1", FKST_TEST_SKIP_DEFAULT_RISK_MOCK = "1" }))
    t.eq(carry.exit_code, 0)
    t.eq(find_raise(carry.raises, "devloop_merge_ready"), nil)
    t.eq(find_raise(carry.raises, "github-proxy.github_pr_comment_request", function(payload)
      return tostring(payload.body or ""):find("review%-carry%-over:v1") ~= nil
    end), nil)
    local reviewing = find_causal_raise(carry, "devloop_reviewing")
    t.is_true(reviewing ~= nil)
    t.eq(reviewing.payload.reviewed_head_sha, nil)
    t.eq(reviewing.payload.version, core.next_review_loop_version(event.version))
    t.eq(count_calls("gh pr merge"), 0)
  end,
}
