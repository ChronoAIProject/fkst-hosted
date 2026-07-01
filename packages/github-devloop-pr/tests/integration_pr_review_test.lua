local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local h = require("tests.devloop_helpers")
local fixtures = require("tests.production_fixture_helpers")
local v_validate_proposal = require("devloop.validators.validate_proposal")
local t = h.t
local core = h.core
local decompose_lib = require("devloop.decompose")
local m_builders = require("devloop.markers.builders")
local action_label = h.action_label
local reason_label = h.reason_label
local has_value = h.has_value
local opts = h.opts
local source_ref = h.source_ref
local issue = h.issue
local reached = h.reached
local unresolved = h.unresolved
local reviewing = h.reviewing
local review_reached = h.review_reached
local review_unresolved = h.review_unresolved
local fixing = h.fixing
local pr_link_marker_for_fix = h.pr_link_marker_for_fix
local review_meta_event = h.review_meta_event
local ai_sentinel = string.char(226, 159, 166) .. "AI:FKST" .. string.char(226, 159, 167)
local verdict_summary_label = "Three-angle verdicts: "
local merge_ready = h.merge_ready
local run_observe = h.run_observe
local run_result = h.run_result
local run_loop = h.run_loop
local run_observe_pr = h.run_observe_pr
local run_review_pr = h.run_review_pr
local run_review_result = h.run_review_result
local run_fix = h.run_fix
local run_review_loop = h.run_review_loop
local run_review_meta = h.run_review_meta
local run_merge = h.run_merge
local json_string = h.json_string
local render_comment = h.render_comment
local default_marker_version = h.default_marker_version
local mock_issue_state = h.mock_issue_state
local state_from_labels = h.state_from_labels
local with_default_state_marker = h.with_default_state_marker
local mock_issue_body = h.mock_issue_body
local mock_issue_result = h.mock_issue_result
local mock_issue_loop = h.mock_issue_loop
local mock_issue_implement_raw = h.mock_issue_implement_raw
local mock_issue_reviewing = h.mock_issue_reviewing
local mock_issue_review = h.mock_issue_review
local mock_issue_fix = h.mock_issue_fix
local mock_issue_fix_for_event = h.mock_issue_fix_for_event
local mock_issue_review_meta = h.mock_issue_review_meta
local mock_issue_merge = h.mock_issue_merge
local merge_comments = h.merge_comments
local mock_pr_origin = h.mock_pr_origin
local mock_pr_origin_for = h.mock_pr_origin_for
local mock_pr_merge = h.mock_pr_merge
local mock_pr_merge_rollup = h.mock_pr_merge_rollup
local mock_merging_comment = h.mock_merging_comment
local mock_pr_merge_command = h.mock_pr_merge_command
local has_call = h.has_call
local mock_issue_close = h.mock_issue_close
local merge_comments_with_merging = h.merge_comments_with_merging
local mock_pr_fix = h.mock_pr_fix
local mock_pr_origin_sequence = h.mock_pr_origin_sequence
local mock_pr_head = h.mock_pr_head
local mock_pr_diff = h.mock_pr_diff
local mock_meta_codex = h.mock_meta_codex
local mock_setup_worktree = h.mock_setup_worktree
local mock_existing_empty_implement_worktree = h.mock_existing_empty_implement_worktree
local mock_existing_empty_implement_worktree_reuse = h.mock_existing_empty_implement_worktree_reuse
local mock_existing_implement_branch = h.mock_existing_implement_branch
local mock_git_commit = h.mock_git_commit
local mock_git_push = h.mock_git_push
local mock_existing_devloop_worktree = h.mock_existing_devloop_worktree
local mock_implement_codex = h.mock_implement_codex
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local mock_issue_view_failure = h.mock_issue_view_failure
local count_calls = h.count_calls
local find_raise = h.find_raise
local find_causal_raise = h.find_causal_raise
local function find_label_raise(raises, target_kind)
  return find_raise(raises, "github-proxy.github_issue_label_request", function(payload)
    return tostring(payload.target_kind or "issue") == tostring(target_kind or "issue")
  end)
end
local function count_label_raises(raises, target_kind)
  local count = 0
  for _, raised in ipairs(raises or {}) do
    local payload = raised.payload or {}
    if raised.queue == "github-proxy.github_issue_label_request"
      and tostring(payload.target_kind or "issue") == tostring(target_kind or "issue") then
      count = count + 1
    end
  end
  return count
end
local function assert_pr_label_guard(payload, expected_state, expected_version)
  t.eq(payload.expected_proposal_id, "github-devloop/issue/owner/repo/42")
  t.eq(payload.expected_state, expected_state)
  t.eq(payload.expected_version, expected_version)
end
local function mock_decompose_child_issue_list(proposal_id, version, pr_number, indexes)
  local repo = base_ids.parse_proposal_id(proposal_id)
  local rendered = {}
  for _, index in ipairs(indexes or {}) do
    table.insert(rendered, string.format(
      '{"number":%d,"title":"Child %d","state":"OPEN","author":{"login":"fkst-test-bot"},"body":"%s","url":"https://github.example/owner/repo/issues/%d"}',
      100 + index,
      index,
      json_string(decompose_lib.decompose_child_marker(core, proposal_id, version, pr_number, index)),
      100 + index
    ))
  end
  t.mock_command(core.gh_issue_list_decompose_children_cmd(repo or "owner/repo", proposal_id), {
    stdout = "[" .. table.concat(rendered, ",") .. "]\n",
    stderr = "",
    exit_code = 0,
  })
end
return {
  test_observe_pr_backpointer_advances_issue_to_reviewing = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    }, "devloop-owner-repo-42-01HY", "def456", "OPEN", "dev", nil, { "fkst-dev:thinking" })
    mock_issue_reviewing({ "fkst-dev:pr-open" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "pr-open", impl_version),
    })
    local result = run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    }, opts("observe-pr-reviewing"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
    t.is_true(comment_raise.payload.body:find("state=\"reviewing\"", 1, true) ~= nil)
    t.eq(comment_raise.payload.handoff.kind, "github-devloop.reviewing")
    t.eq(comment_raise.payload.handoff.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(comment_raise.payload.handoff.pr_number, 7)
    t.eq(comment_raise.payload.handoff.version, impl_version)
    t.eq(find_label_raise(result.raises, "issue"), nil)
    t.eq(find_label_raise(result.raises, "pr"), nil)
    local handoff = h.run_comment_handoff_from_request(comment_raise.payload, "IC_devloop_reviewing_2", "observe-pr-reviewing-label-handoff")
    local pr_label_raise = find_label_raise(handoff.raises, "pr")
    t.is_true(pr_label_raise ~= nil)
    t.eq(pr_label_raise.payload.target_kind, "pr")
    t.eq(pr_label_raise.payload.target_number, 7)
    t.eq(pr_label_raise.payload.add_labels[1], "fkst-dev:reviewing")
    t.eq(pr_label_raise.payload.label_colors["fkst-dev:reviewing"], "5319E7")
    assert_pr_label_guard(pr_label_raise.payload, "reviewing", impl_version)
    local reviewing_raise = find_causal_raise(result, "devloop_reviewing")
    t.eq(reviewing_raise.payload.schema, "github-devloop.reviewing.v1")
    t.eq(reviewing_raise.payload.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(reviewing_raise.payload.pr_number, 7)
    t.eq(reviewing_raise.payload.version, impl_version)
    t.eq(reviewing_raise.payload.reviewing_hand_off.comment_id, "IC_devloop_reviewing_1")
    t.eq(reviewing_raise.payload.reviewing_hand_off.marker_version, impl_version)
  end,
  test_observe_pr_reconciles_regressed_label_to_reviewing_marker = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    }, "devloop-owner-repo-42-01HY", "def456", "OPEN", "dev", nil, { "fkst-dev:thinking" })
    mock_issue_reviewing({ "fkst-dev:pr-open" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })
    local result = run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    }, opts("observe-pr-reconcile-reviewing"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local pr_label_raise = find_label_raise(result.raises, "pr")
    t.is_true(pr_label_raise ~= nil)
    t.eq(find_label_raise(result.raises, "issue"), nil)
    t.eq(pr_label_raise.payload.add_labels[1], "fkst-dev:reviewing")
    t.eq(pr_label_raise.payload.label_colors["fkst-dev:reviewing"], "5319E7")
    t.eq(pr_label_raise.payload.target_number, 7)
    assert_pr_label_guard(pr_label_raise.payload, "reviewing", impl_version)
  end,
  test_observe_pr_does_not_reconcile_issue_label_from_pr_fixing_state = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local fix_version = core.next_fix_version(impl_version)
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "fixing", fix_version),
    }, "devloop-owner-repo-42-01HY", "def456", "OPEN", "dev", nil, { "fkst-dev:pr-open" })
    mock_issue_reviewing({ "fkst-dev:pr-open" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "pr-open", impl_version),
    })

    local result = run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    }, opts("observe-pr-no-issue-label-from-pr-fixing"))

    t.eq(result.exit_code, 0)
    t.eq(count_label_raises(result.raises, "issue"), 0)
    local pr_label_raise = find_label_raise(result.raises, "pr")
    if pr_label_raise ~= nil then
      t.eq(pr_label_raise.payload.target_kind, "pr")
      t.eq(pr_label_raise.payload.target_number, 7)
      t.eq(pr_label_raise.payload.add_labels[1], "fkst-dev:fixing")
      t.eq(pr_label_raise.payload.label_colors["fkst-dev:fixing"], "D93F0B")
      assert_pr_label_guard(pr_label_raise.payload, "fixing", fix_version)
    end
  end,
  test_observe_pr_removes_stale_reviewing_label_from_blocked_pr_marker = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker("github-devloop/issue/owner/repo/42", "blocked", impl_version .. "/blocked"),
      decompose_lib.decomposed_marker(core, "github-devloop/issue/owner/repo/42", impl_version .. "/blocked", 7, 1),
    }, "devloop-owner-repo-42-01HY", "def456", "OPEN", "dev", nil, { "fkst-dev:reviewing" })
    mock_decompose_child_issue_list("github-devloop/issue/owner/repo/42", impl_version .. "/blocked", 7, {})

    local result = run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    }, opts("observe-pr-reconcile-blocked-stale-reviewing"))

    t.eq(result.exit_code, 0)
    local pr_label_raise = find_label_raise(result.raises, "pr")
    t.eq(pr_label_raise.payload.target_number, 7)
    t.eq(pr_label_raise.payload.add_labels[1], "fkst-dev:blocked")
    t.eq(pr_label_raise.payload.label_colors["fkst-dev:blocked"], "1B1F23")
    assert_pr_label_guard(pr_label_raise.payload, "blocked", impl_version .. "/blocked")
    t.is_true(has_value(pr_label_raise.payload.remove_labels, "fkst-dev:reviewing"))
  end,
  test_observe_pr_reraises_merge_ready_for_poll_self_heal = function()
    local event = merge_ready()
    mock_pr_origin({
      m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
    })
    mock_issue_reviewing({ "fkst-dev:merge-ready" }, merge_comments(event))
    local result = run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    }, opts("observe-pr-merge-ready-self-heal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local merge_raise = find_raise(result.raises, "devloop_merge_ready")
    t.eq(find_label_raise(result.raises, "pr").payload.add_labels[1], "fkst-dev:merge-ready")
    t.eq(merge_raise.payload.schema, "github-devloop.merge-ready.v1")
    t.eq(merge_raise.payload.proposal_id, event.proposal_id)
    t.eq(merge_raise.payload.pr_number, event.pr_number)
    t.eq(merge_raise.payload.version, event.version)
    t.eq(merge_raise.payload.reviewed_head_sha, event.reviewed_head_sha)
  end,
  test_observe_pr_reraises_merging_for_poll_self_heal = function()
    local event = merge_ready()
    local comments = merge_comments(event)
    table.insert(comments, core.state_marker(event.proposal_id, "merging", event.version))
    mock_pr_origin({
      m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
    })
    mock_issue_reviewing({ "fkst-dev:merging" }, comments)
    local result = run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    }, opts("observe-pr-merging-self-heal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local merge_raise = find_raise(result.raises, "devloop_merge_ready")
    t.eq(find_label_raise(result.raises, "pr").payload.add_labels[1], "fkst-dev:merging")
    t.eq(merge_raise.payload.schema, "github-devloop.merge-ready.v1")
    t.eq(merge_raise.payload.proposal_id, event.proposal_id)
    t.eq(merge_raise.payload.pr_number, event.pr_number)
    t.eq(merge_raise.payload.version, event.version)
    t.eq(merge_raise.payload.reviewed_head_sha, event.reviewed_head_sha)
  end,
  test_observe_pr_idempotent_reviewing_marker_reraises_until_review_result_visible = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_reviewing({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })
    local first = run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    }, opts("observe-pr-reviewing-self-heal"))
    t.eq(first.exit_code, 0)
    local reviewing_raise = find_causal_raise(first, "devloop_reviewing")
    t.is_true(reviewing_raise ~= nil)
    t.eq(find_label_raise(first.raises, "pr").payload.add_labels[1], "fkst-dev:reviewing")
    t.eq(reviewing_raise.payload.version, impl_version .. "/review-loop/1")

    local review_id = devloop_base.pr_review_proposal_id("owner/repo", 7, reviewing_raise.payload.version, "def456")
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_reviewing({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", reviewing_raise.payload.version),
      m_builders.review_result_marker(core, review_id, "github-devloop/issue/owner/repo/42", "approve", "consensus:" .. review_id .. "/review"),
    })
    local reviewed = run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:04Z",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    }, opts("observe-pr-reviewing-reviewed"))
    t.eq(reviewed.exit_code, 0)
    t.eq(find_label_raise(reviewed.raises, "issue"), nil)
    t.eq(find_label_raise(reviewed.raises, "pr").payload.add_labels[1], "fkst-dev:reviewing")
  end,

  test_observe_pr_reviewing_self_heal_uses_canonical_fix_round_version = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local fix_round_version = core.next_fix_version(impl_version)
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    }, "devloop-owner-repo-42-01HY", "feedface")
    mock_issue_reviewing({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", fix_round_version),
    })
    local result = run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:05Z",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    }, opts("observe-pr-reviewing-fix-round-self-heal"))
    t.eq(result.exit_code, 0)
    local reviewing_raise = find_causal_raise(result, "devloop_reviewing")
    t.is_true(reviewing_raise ~= nil)
    local label_raise = find_label_raise(result.raises, "pr")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:reviewing")
    t.eq(label_raise.payload.label_colors["fkst-dev:reviewing"], "5319E7")
    assert_pr_label_guard(label_raise.payload, "reviewing", fix_round_version)
    t.eq(reviewing_raise.payload.version, fix_round_version .. "/review-loop/1")
    mock_bot_env()
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", reviewing_raise.payload.version),
    })
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    }, "devloop-owner-repo-42-01HY", "feedface")
    local review = run_review_pr(reviewing_raise.payload, opts("observe-pr-reviewing-fix-round-rereview"))
    t.eq(review.exit_code, 0)
    t.eq(#review.raises, 1)
    local proposal = find_raise(review.raises, "consensus.proposal").payload
    t.eq(proposal.proposal_id, devloop_base.pr_review_proposal_id("owner/repo", 7, reviewing_raise.payload.version, "feedface"))
    t.is_nil(proposal.body:find("+fixed by replay", 1, true))
    t.is_true(proposal.content_fetch:find("runtime-cache:", 1, true) == 1)
  end,
  test_observe_pr_without_visible_backpointer_uses_pr_native_origin = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local branch = devloop_base.implement_branch("owner/repo", "42", impl_version)
    mock_pr_origin({}, branch)
    local result = run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    }, opts("observe-pr-backpointer-pending"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
    t.eq(find_label_raise(result.raises, "pr"), nil)
  end,
  test_observe_pr_non_devloop_branch_without_visible_backpointer_uses_pr_native_origin = function()
    mock_pr_origin({}, "feature/unrelated")
    local result = run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    }, opts("observe-pr-backpointer-foreign"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
    t.eq(find_label_raise(result.raises, "pr"), nil)
  end,
  test_observe_pr_closed_pr_redrives_ready_without_advancing_to_reviewing = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    }, "devloop-owner-repo-42-01HY", "def456", "CLOSED")
    mock_issue_reviewing({ "fkst-dev:pr-open" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "pr-open", impl_version),
    })
    local result = run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    }, opts("observe-pr-closed"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
    local terminal = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(terminal ~= nil)
    t.is_true(terminal.payload.body:find('state="closed-unmerged"', 1, true) ~= nil)
    t.is_true(terminal.payload.handoff ~= nil)
    t.eq(terminal.payload.handoff.kind, "github-devloop.closed_unmerged")
    t.eq(find_label_raise(result.raises, "pr"), nil)
    local handoff = h.run_comment_handoff_from_request(terminal.payload, "IC_closed_unmerged_1", "closed-unmerged-comment-handoff")
    local label = find_label_raise(handoff.raises, "pr")
    t.is_true(label ~= nil)
    t.eq(label.payload.expected_state, "closed-unmerged")
    t.eq(label.payload.marker_guard.expected.state, "closed-unmerged")
  end,
  test_observe_pr_ignores_forged_backpointer_and_uses_pr_native_origin = function()
    mock_pr_origin({
      {
        body = m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", "v1", "dev"),
        author_login = "ordinary-user",
      },
    })
    local result = run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
      source_ref = {
        kind = "external",
        ref = "owner/repo#pr/7",
      },
    }, opts("observe-pr-forged"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
    t.eq(find_label_raise(result.raises, "pr"), nil)
  end,
  test_review_pr_builds_pr_review_consensus_proposal = function()
    local event = reviewing()
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", event.version),
    }, {
      title = "Implement decision recorder",
      body = "Issue context",
    })
    mock_pr_origin_sequence({
      { head = "devloop-owner-repo-42-01HY", head_sha = "def456" },
    })

    local result = run_review_pr(event, opts("review-pr-proposal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus.proposal")
    local proposal = result.raises[1].payload
    t.eq(proposal.schema, "consensus.proposal.v1")
    t.eq(proposal.proposal_id, devloop_base.pr_review_proposal_id("owner/repo", 7, event.version, "def456"))
    t.eq(proposal.source_ref.ref, "owner/repo#pr/7")
    t.is_true(#proposal.body < 512)
    t.is_nil(proposal.body:find("BEGIN UNTRUSTED ISSUE DATA", 1, true))
    t.is_nil(proposal.body:find("+return true", 1, true))
    t.is_true(proposal.body:find("Reviewed PR head: def456", 1, true) ~= nil)
    t.is_true(proposal.content_fetch:find("runtime-cache:", 1, true) == 1)
    t.eq(v_validate_proposal.validate_proposal(core, proposal), true)
    t.eq(count_calls("gh pr diff"), 2)
  end,

  test_review_pr_gate_reject_reached_routes_to_fixing = function()
    local event = reviewing()
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", event.version),
    }, {
      title = "Implement decision recorder",
      body = "Issue context",
    })
    mock_pr_origin_sequence({
      { head = "devloop-owner-repo-42-01HY", head_sha = "def456" },
    })

    local review = run_review_pr(event, opts("review-pr-gate-reject-link"))
    t.eq(review.exit_code, 0)
    t.eq(#review.raises, 1)
    local proposal = find_raise(review.raises, "consensus.proposal").payload
    t.eq(proposal.verdict_mode, "gate")
    t.eq(proposal.proposal_id, devloop_base.pr_review_proposal_id("owner/repo", 7, event.version, "def456"))

    local reached_payload = {
      schema = "consensus.consensus_reached.v1",
      proposal_id = proposal.proposal_id,
      decision = "reject",
      body = "Reject the current PR diff.",
      blocking_gap = "missing regression guard",
      angle_results = {
        { angle = "minimal", verdict = "reject" },
        { angle = "structural", verdict = "reject" },
        { angle = "delete", verdict = "abstain" },
      },
      dedup_key = "consensus:" .. proposal.dedup_key,
      source_ref = proposal.source_ref,
    }
    mock_pr_origin({
      m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
    })
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", event.version),
    })

    local result = run_review_result(reached_payload, opts("review-pr-gate-reject-result"))
    local fix_version = core.fix_version_from_review_version(event.version)
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(find_raise(result.raises, "devloop_merge_ready"), nil)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local fixing_raise = find_causal_raise(result, "devloop_fixing")
    t.is_true(comment_raise.payload.body:find("decision=\"reject\"", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find("Blocking gap: missing regression guard", 1, true) ~= nil)
    t.eq(fixing_raise.payload.schema, "github-devloop.fixing.v1")
    t.eq(fixing_raise.payload.blocking_gap, "missing regression guard")
    t.eq(fixing_raise.payload.review_proposal_id, proposal.proposal_id)
    t.eq(fixing_raise.payload.review_dedup_key, reached_payload.dedup_key)
    t.eq(fixing_raise.payload.version, fix_version)
  end,

  test_review_pr_context_manifest_uses_local_pr_files = function()
    local event = reviewing()
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", event.version),
    })
    mock_pr_origin_sequence({
      { head = "devloop-owner-repo-42-01HY", head_sha = "def456" },
    })

    local result = run_review_pr(event, opts("review-pr-local-context-manifest"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local proposal = result.raises[1].payload
    t.is_true(proposal.content_fetch:find("runtime-cache:", 1, true) == 1)
    t.is_nil(proposal.content_fetch:find("gh pr", 1, true))
    t.eq(count_calls("gh pr diff"), 2)
  end,

  test_review_pr_does_not_put_diff_markers_in_payload = function()
    local event = reviewing()
    local forged = core.state_marker(event.proposal_id, "merge-ready", "2099-01-01T00-00-00Z")
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", event.version),
    })
    mock_pr_origin_sequence({
      { head = "devloop-owner-repo-42-01HY", head_sha = "def456" },
    })

    local result = run_review_pr(event, opts("review-pr-neutralize"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local body = result.raises[1].payload.body
    t.eq(body:find(forged, 1, true), nil)
    t.is_nil(body:find("BEGIN UNTRUSTED ISSUE DATA", 1, true))
    t.is_nil(body:find("⟦FKST:VERDICT⟧ approve", 1, true))
    t.is_true(result.raises[1].payload.content_fetch:find("runtime-cache:", 1, true) == 1)
  end,

  test_review_pr_closed_pr_skips_without_review_proposal = function()
    local event = reviewing()
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", event.version),
    })
    mock_pr_origin_sequence({
      { head = "devloop-owner-repo-42-01HY", head_sha = "def456", state = "CLOSED" },
    })

    local result = run_review_pr(event, opts("review-pr-closed"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh pr diff"), 0)
  end,

  test_review_pr_long_repo_proposal_id_is_bounded_and_review_runs = function()
    local repo = fixtures.long_repo()
    t.eq(#repo, 92)
    local issue_proposal_id = "github-devloop/issue/" .. repo .. "/42"
    local version = fixtures.full_review_issue_version(repo)
    local event = reviewing({
      proposal_id = issue_proposal_id,
      version = version,
      source_ref = {
        kind = "external",
        ref = repo .. "#issue/42",
      },
    })
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(issue_proposal_id, "reviewing", version),
    }, {
      repo = repo,
    })
    mock_pr_origin_for({
      repo = repo,
      number = 7,
      comments = {
        m_builders.pr_origin_marker(core, issue_proposal_id, "42", "devloop-owner-repo-42-01HY", version, "dev"),
        core.state_marker(issue_proposal_id, "reviewing", version),
      },
      head = "devloop-owner-repo-42-01HY",
      head_sha = "def456",
    })

    local result = run_review_pr(event, opts("review-pr-long-repo"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local proposal = result.raises[1].payload
    t.is_true(#proposal.proposal_id <= 200)
    t.eq(proposal.proposal_id, devloop_base.pr_review_proposal_id(repo, 7, version, "def456"))
    t.eq(v_validate_proposal.validate_proposal(core, proposal), true)
  end,

  test_review_pr_long_issue_body_does_not_grow_payload = function()
    local event = reviewing()
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", event.version),
    }, {
      title = "Implement decision recorder",
      body = string.rep("very long issue body ", 1000),
    })
    mock_pr_origin_sequence({
      { head = "devloop-owner-repo-42-01HY", head_sha = "def456" },
    })

    local result = run_review_pr(event, opts("review-pr-long-issue-keeps-diff"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local body = result.raises[1].payload.body
    t.is_true(#body < 512)
    t.is_nil(body:find("very long issue body", 1, true))
    t.is_nil(body:find("+DIFF_SENTINEL_MUST_SURVIVE", 1, true))
    t.is_true(result.raises[1].payload.content_fetch:find("runtime-cache:", 1, true) == 1)
  end,

  test_review_pr_stale_idempotent_and_not_reviewing_skip_or_retry = function()
    local event = reviewing()
    local newer = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", newer),
    })
    local stale = run_review_pr(event, opts("review-pr-stale-version"))
    t.eq(stale.exit_code, 0)
    t.eq(#stale.raises, 0)
    t.eq(count_calls("gh pr diff"), 0)

    mock_issue_review({ "fkst-dev:merge-ready" }, {
      core.state_marker(event.proposal_id, "merge-ready", event.version),
    })
    local advanced = run_review_pr(event, opts("review-pr-advanced"))
    t.eq(advanced.exit_code, 0)
    t.eq(#advanced.raises, 0)
    t.eq(count_calls("gh pr diff"), 0)

    mock_issue_review({ "fkst-dev:pr-open" }, {
      core.state_marker(event.proposal_id, "pr-open", event.version),
    })
    local lagged_predecessor = run_review_pr(event, opts("review-pr-lagged-predecessor"))
    t.eq(lagged_predecessor.exit_code, 1)
    t.eq(#lagged_predecessor.raises, 0)
    t.eq(count_calls("gh pr diff"), 0)

    mock_issue_review({ "fkst-dev:enabled" }, {})
    local pending = run_review_pr(event, opts("review-pr-pending-marker"))
    t.eq(pending.exit_code, 1)
    t.eq(#pending.raises, 0)
    t.eq(count_calls("gh pr diff"), 0)
  end,

  test_review_pr_accepts_verified_durable_reviewing_hand_off_before_marker_visibility = function()
    local event = reviewing()
    event.reviewing_hand_off = {
      kind = "own-state-marker",
      proposal_id = event.proposal_id,
      state = "reviewing",
      marker_version = event.version,
      event_version = event.version,
      stage_rank = core.stage_rank("reviewing"),
      comment_id = "IC_reviewing_1",
    }
    mock_issue_review({ "fkst-dev:reviewing" }, {})
    mock_pr_origin_sequence({
      {
        comments = {
          m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
        },
        head = "devloop-owner-repo-42-01HY",
        head_sha = "def456",
      },
    })
    t.mock_command("gh api --method GET 'repos/owner/repo/issues/comments/IC_reviewing_1'", {
      stdout = '{"body":"' .. json_string(core.state_marker(event.proposal_id, "reviewing", event.version)) .. '","user":{"login":"fkst-test-bot"}}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = run_review_pr(event, opts("review-pr-durable-reviewing-hand-off"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus.proposal")
    t.eq(count_calls("repos/owner/repo/issues/comments/IC_reviewing_1"), 1)
    t.eq(count_calls("gh pr diff"), 2)
  end,

}
