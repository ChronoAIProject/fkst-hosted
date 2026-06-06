local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local action_label = h.action_label
local reason_label = h.reason_label
local has_value = h.has_value
local opts = h.opts
local source_ref = h.source_ref
local issue = h.issue
local reached = h.reached
local unresolved = h.unresolved
local stuck = h.stuck
local ready = h.ready
local reviewing = h.reviewing
local review_reached = h.review_reached
local review_unresolved = h.review_unresolved
local fixing = h.fixing
local pr_link_marker_for_fix = h.pr_link_marker_for_fix
local review_meta_event = h.review_meta_event
local merge_ready = h.merge_ready
local run_observe = h.run_observe
local run_result = h.run_result
local run_loop = h.run_loop
local run_meta = h.run_meta
local run_implement = h.run_implement
local run_open_pr = h.run_open_pr
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
local mock_issue_meta = h.mock_issue_meta
local mock_issue_implement = h.mock_issue_implement
local mock_issue_implement_raw = h.mock_issue_implement_raw
local mock_issue_open_pr = h.mock_issue_open_pr
local mock_issue_reviewing = h.mock_issue_reviewing
local mock_issue_review = h.mock_issue_review
local mock_issue_fix = h.mock_issue_fix
local mock_issue_fix_for_event = h.mock_issue_fix_for_event
local mock_issue_review_meta = h.mock_issue_review_meta
local mock_issue_merge = h.mock_issue_merge
local merge_comments = h.merge_comments
local mock_pr_origin = h.mock_pr_origin
local review_json = h.review_json
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
local mock_branch_exists = h.mock_branch_exists
local mock_meta_codex = h.mock_meta_codex
local mock_setup_worktree = h.mock_setup_worktree
local deterministic_branch_for = h.deterministic_branch_for
local mock_fresh_implement_worktree = h.mock_fresh_implement_worktree
local mock_existing_empty_implement_worktree = h.mock_existing_empty_implement_worktree
local mock_existing_empty_implement_worktree_reuse = h.mock_existing_empty_implement_worktree_reuse
local mock_existing_implement_branch = h.mock_existing_implement_branch
local mock_git_commit = h.mock_git_commit
local mock_git_push = h.mock_git_push
local mock_existing_devloop_worktree = h.mock_existing_devloop_worktree
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local mock_issue_view_failure = h.mock_issue_view_failure
local count_calls = h.count_calls
local find_raise = h.find_raise

return {
  test_fix_authorized_write_pushes_and_marks_reviewing_new_head = function()
    local event = fixing()
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      {
        proposal_id = event.review_proposal_id,
        decision = "reject",
        body = "Reject because parser must fail closed.",
        dedup_key = event.review_dedup_key,
        source_ref = { kind = "external", ref = "owner/repo#pr/7" },
      },
      event.source_ref
    ).body
    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", branch, event.version)
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing", "fkst-dev:fix-authorized" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-packages-test/github-devloop/runtime", stderr = "", exit_code = 0 })
    t.mock_command("git worktree list --porcelain", {
      stdout = "worktree /tmp/fix-worktree\nHEAD def456\nbranch refs/heads/" .. branch .. "\n\n",
      stderr = "",
      exit_code = 0,
    })
    mock_implement_codex(0, "fixed review feedback")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing", "fkst-dev:fix-authorized" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    mock_git_push(branch)
    mock_pr_fix({ origin_marker }, branch, "feedface")

    local result = run_fix(event, opts("fix-authorized-write", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
	    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
	    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request")
	    local reviewing_raise = find_raise(result.raises, "devloop_reviewing")
    local expected_version = core.next_fix_version(event.version)
	    t.eq(label_raise.payload.add_labels[1], "fkst-dev:reviewing")
	    t.is_true(has_value(label_raise.payload.remove_labels, "fkst-dev:fix-authorized"))
	    t.is_true(has_value(label_raise.payload.remove_labels, "fkst-dev:merge-authorized"))
	    t.is_true(comment_raise.payload.body:find(core.fix_marker(event.proposal_id, event.review_proposal_id, event.review_dedup_key, "def456", "feedface"), 1, true) ~= nil)
    local current = core.current_state({
      core.state_marker(event.proposal_id, "fixing", event.version),
      comment_raise.payload.body,
    }, event.proposal_id)
    t.eq(current.state, "reviewing")
    t.eq(current.version, expected_version)
    t.eq(reviewing_raise.payload.version, expected_version)
	    t.eq(count_calls("git push origin"), 1)

    mock_bot_env()
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      comment_raise.payload.body,
    })
    local origin_marker_for_review = core.pr_origin_marker(event.proposal_id, "42", branch, event.version)
    mock_pr_origin({ origin_marker_for_review }, branch, "feedface")
    mock_pr_diff("diff --git a/packages/github-devloop/core.lua b/packages/github-devloop/core.lua\n+fixed again\n")
    mock_pr_origin({ origin_marker_for_review }, branch, "feedface")

    local review_result = run_review_pr(reviewing_raise.payload, opts("fix-authorized-write-rereview"))
    t.eq(review_result.exit_code, 0)
    t.eq(#review_result.raises, 1)
    local proposal = find_raise(review_result.raises, "consensus.proposal").payload
    t.eq(proposal.proposal_id, core.pr_review_proposal_id("owner/repo", 7, expected_version, "feedface"))
    t.is_true(proposal.body:find("+fixed again", 1, true) ~= nil)
	  end,

  test_fix_marker_lag_retries_then_visible_marker_runs = function()
    local event = fixing()
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      { proposal_id = event.review_proposal_id, decision = "reject", body = "Reject.", dedup_key = event.review_dedup_key, source_ref = { kind = "external", ref = "owner/repo#pr/7" } },
      event.source_ref
    ).body

    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:enabled", "fkst-dev:fix-authorized" }, {
      reject_comment,
    }, branch, event.version)
    local pending = run_fix(event, opts("fix-marker-lag", { FKST_GITHUB_WRITE = "1" }))
    t.eq(pending.exit_code, 1)
    t.eq(#pending.raises, 0)
    t.eq(count_calls("codex exec"), 0)

    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", branch, event.version)
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing", "fkst-dev:fix-authorized" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-packages-test/github-devloop/runtime", stderr = "", exit_code = 0 })
    t.mock_command("git worktree list --porcelain", {
      stdout = "worktree /tmp/fix-worktree\nHEAD def456\nbranch refs/heads/" .. branch .. "\n\n",
      stderr = "",
      exit_code = 0,
    })
    mock_implement_codex(0, "fixed after marker became visible")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing", "fkst-dev:fix-authorized" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    mock_git_push(branch)
    mock_pr_fix({ origin_marker }, branch, "feedface")

    local visible = run_fix(event, opts("fix-marker-visible", { FKST_GITHUB_WRITE = "1" }))
    t.eq(visible.exit_code, 0)
    t.eq(#visible.raises, 3)
    t.eq(find_raise(visible.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:reviewing")
  end,

  test_fix_new_round_is_pending_against_old_reviewing_when_fixing_marker_lags = function()
    local review_version = reviewing().version
    local event = fixing({
      version = core.fix_version_from_review_version(review_version),
    })
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      { proposal_id = event.review_proposal_id, decision = "reject", body = "Reject.", dedup_key = event.review_dedup_key, source_ref = { kind = "external", ref = "owner/repo#pr/7" } },
      event.source_ref
    ).body

    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:reviewing", "fkst-dev:fix-authorized" }, {
      core.state_marker(event.proposal_id, "reviewing", review_version),
      reject_comment,
    }, branch, review_version)

    local pending = run_fix(event, opts("fix-new-round-marker-lag", { FKST_GITHUB_WRITE = "1" }))
    t.eq(pending.exit_code, 1)
    t.eq(#pending.raises, 0)
    t.eq(count_calls("codex exec"), 0)
  end,

  test_fix_skips_when_target_reviewing_round_is_already_current = function()
    local event = fixing()
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local reviewing_version = core.next_fix_version(event.version)
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      { proposal_id = event.review_proposal_id, decision = "reject", body = "Reject.", dedup_key = event.review_dedup_key, source_ref = { kind = "external", ref = "owner/repo#pr/7" } },
      event.source_ref
    ).body
    mock_bot_env()
    mock_issue_fix_for_event(event, { "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", reviewing_version),
      reject_comment,
    }, branch, event.version)

    local result = run_fix(event, opts("fix-idempotent-reviewing"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("codex exec"), 0)
  end,

  test_fix_missing_human_gate_dry_run_no_advance = function()
    local event = fixing()
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      { proposal_id = event.review_proposal_id, decision = "reject", body = "Reject.", dedup_key = event.review_dedup_key, source_ref = { kind = "external", ref = "owner/repo#pr/7" } },
      event.source_ref
    ).body
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)

    local result = run_fix(event, opts("fix-missing-gate", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("codex exec"), 0)
    t.eq(count_calls("git push origin"), 0)
  end,

  test_fix_runs_after_authorization_is_added = function()
    local event = fixing()
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      { proposal_id = event.review_proposal_id, decision = "reject", body = "Reject.", dedup_key = event.review_dedup_key, source_ref = { kind = "external", ref = "owner/repo#pr/7" } },
      event.source_ref
    ).body
    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", branch, event.version)

    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    local unauthorized = run_fix(event, opts("fix-auth-later-first", { FKST_GITHUB_WRITE = "1" }))
    t.eq(unauthorized.exit_code, 1)
    t.eq(count_calls("codex exec"), 0)

    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing", "fkst-dev:fix-authorized" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-packages-test/github-devloop/runtime", stderr = "", exit_code = 0 })
    t.mock_command("git worktree list --porcelain", {
      stdout = "worktree /tmp/fix-worktree\nHEAD def456\nbranch refs/heads/" .. branch .. "\n\n",
      stderr = "",
      exit_code = 0,
    })
    mock_implement_codex(0, "fixed review feedback")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing", "fkst-dev:fix-authorized" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    mock_git_push(branch)
    mock_pr_fix({ origin_marker }, branch, "feedface")

    local authorized = run_fix(event, opts("fix-auth-later-second", { FKST_GITHUB_WRITE = "1" }))
    t.eq(authorized.exit_code, 0)
    t.eq(#authorized.raises, 3)
    t.eq(find_raise(authorized.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:reviewing")
    t.eq(find_raise(authorized.raises, "devloop_reviewing").payload.version, core.next_fix_version(event.version))
    t.eq(count_calls("git push origin"), 1)
  end,

  test_second_round_fix_uses_pr_origin_branch_not_recomputed_version_branch = function()
    local first_event = fixing()
    local first_branch = core.implement_branch("owner/repo", "42", first_event.version)
    local second_version = core.next_fix_version(first_event.version)
    local second_review_version = first_event.version
    local second_event = fixing({
      version = second_version,
      review_proposal_id = core.pr_review_proposal_id("owner/repo", 7, second_review_version, "feedface"),
      review_dedup_key = "consensus:" .. core.pr_review_proposal_id("owner/repo", 7, second_review_version, "feedface") .. "/review",
      reviewed_head_sha = "feedface",
      dedup_key = "fixing/github-devloop/issue/owner/repo/42/v2",
    })
    local recomputed_branch = core.implement_branch("owner/repo", "42", second_event.version)
    t.eq(first_branch ~= recomputed_branch, true)
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      second_event.proposal_id,
      second_event.version,
      {
        proposal_id = second_event.review_proposal_id,
        decision = "reject",
        body = "Reject second round.",
        dedup_key = second_event.review_dedup_key,
        source_ref = { kind = "external", ref = "owner/repo#pr/7" },
      },
      second_event.source_ref
    ).body
    local origin_marker = core.pr_origin_marker(second_event.proposal_id, "42", first_branch, first_event.version)
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(second_event, { "fkst-dev:fixing", "fkst-dev:fix-authorized" }, {
      core.state_marker(second_event.proposal_id, "fixing", second_event.version),
      reject_comment,
    }, first_branch, first_event.version)
    mock_pr_fix({ origin_marker }, first_branch, "feedface")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-packages-test/github-devloop/runtime", stderr = "", exit_code = 0 })
    t.mock_command("git worktree list --porcelain", {
      stdout = "worktree /tmp/fix-worktree\nHEAD feedface\nbranch refs/heads/" .. first_branch .. "\n\n",
      stderr = "",
      exit_code = 0,
    })
    mock_implement_codex(0, "fixed second-round review feedback")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("baddad", first_branch)
    mock_write_env("1")
    mock_issue_fix_for_event(second_event, { "fkst-dev:fixing", "fkst-dev:fix-authorized" }, {
      core.state_marker(second_event.proposal_id, "fixing", second_event.version),
      reject_comment,
    }, first_branch, first_event.version)
    mock_pr_fix({ origin_marker }, first_branch, "feedface")
    mock_git_push(first_branch)
    mock_pr_fix({ origin_marker }, first_branch, "baddad")

    local result = run_fix(second_event, opts("fix-second-round-origin-branch", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:reviewing")
    t.eq(find_raise(result.raises, "devloop_reviewing").payload.version, core.next_fix_version(second_version))
    t.eq(count_calls("git push origin"), 1)
    t.eq(count_calls(recomputed_branch), 0)
  end,

  test_fix_push_then_crash_replay_self_heals_reviewing_marker = function()
    local event = fixing()
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      { proposal_id = event.review_proposal_id, decision = "reject", body = "Reject.", dedup_key = event.review_dedup_key, source_ref = { kind = "external", ref = "owner/repo#pr/7" } },
      event.source_ref
    ).body
    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", branch, event.version)

    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing", "fkst-dev:fix-authorized" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_write_env("1")
    mock_pr_fix({ origin_marker }, branch, "feedface")
    t.mock_command("rev-parse --verify refs/heads/", {
      stdout = "feedface\n",
      stderr = "",
      exit_code = 0,
    })

    local result = run_fix(event, opts("fix-push-crash-self-heal", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:reviewing")
    t.eq(core.current_state({ find_raise(result.raises, "github-proxy.github_issue_comment_request").payload.body }, event.proposal_id).version, core.next_fix_version(event.version))
    t.eq(find_raise(result.raises, "devloop_reviewing").payload.version, core.next_fix_version(event.version))
    t.eq(count_calls("codex exec"), 0)
    t.eq(count_calls("git push origin"), 0)
  end,

  test_fix_missing_head_repository_fails_closed = function()
    local event = fixing()
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      { proposal_id = event.review_proposal_id, decision = "reject", body = "Reject.", dedup_key = event.review_dedup_key, source_ref = { kind = "external", ref = "owner/repo#pr/7" } },
      event.source_ref
    ).body
    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", branch, event.version)

    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing", "fkst-dev:fix-authorized" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_write_env("1")
    t.mock_command("--json headRefName,headRefOid,state,comments,headRepository,headRepositoryOwner,isCrossRepository", {
      stdout = string.format(
        '{"headRefName":"%s","headRefOid":"def456","state":"OPEN","comments":[%s],"isCrossRepository":false}\n',
        json_string(branch),
        render_comment(origin_marker)
      ),
      stderr = "",
      exit_code = 0,
    })

    local result = run_fix(event, opts("fix-missing-head-repository", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("codex exec"), 0)
  end,

  test_fix_no_changes_escalates_to_review_meta = function()
    local event = fixing()
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      { proposal_id = event.review_proposal_id, decision = "reject", body = "Reject.", dedup_key = event.review_dedup_key, source_ref = { kind = "external", ref = "owner/repo#pr/7" } },
      event.source_ref
    ).body
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing", "fkst-dev:fix-authorized" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_write_env("1")
    mock_pr_fix({ core.pr_origin_marker(event.proposal_id, "42", branch, event.version) }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-packages-test/github-devloop/runtime", stderr = "", exit_code = 0 })
    t.mock_command("git worktree list --porcelain", {
      stdout = "worktree /tmp/fix-worktree\nHEAD def456\nbranch refs/heads/" .. branch .. "\n\n",
      stderr = "",
      exit_code = 0,
    })
    mock_implement_codex(0, "No viable fix.")
    mock_git_status("")
    t.mock_command("rev-list --count", {
      stdout = "0\n",
      stderr = "",
      exit_code = 0,
    })

    local result = run_fix(event, opts("fix-no-changes-review-meta", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:review-meta")
    local comment_body = find_raise(result.raises, "github-proxy.github_issue_comment_request").payload.body
    t.is_true(comment_body:find("github-devloop fix escalated to review-meta: no-fix", 1, true) ~= nil)
    t.eq(comment_body:find("fkst:github-devloop:review-meta:v1", 1, true), nil)
    t.eq(find_raise(result.raises, "devloop_review_meta").payload.schema, "github-devloop.review-meta.v1")
  end,

  test_fix_clean_worktree_with_existing_ahead_commit_reuses_it = function()
    local event = fixing()
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      { proposal_id = event.review_proposal_id, decision = "reject", body = "Reject.", dedup_key = event.review_dedup_key, source_ref = { kind = "external", ref = "owner/repo#pr/7" } },
      event.source_ref
    ).body
    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", branch, event.version)
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing", "fkst-dev:fix-authorized" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-packages-test/github-devloop/runtime", stderr = "", exit_code = 0 })
    t.mock_command("git worktree list --porcelain", {
      stdout = "worktree /tmp/fix-worktree\nHEAD feedface\nbranch refs/heads/" .. branch .. "\n\n",
      stderr = "",
      exit_code = 0,
    })
    mock_implement_codex(0, "Fix commit already exists.")
    mock_git_status("")
    t.mock_command("rev-list --count", {
      stdout = "1\n",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("rev-parse --verify refs/heads/", {
      stdout = "feedface\n",
      stderr = "",
      exit_code = 0,
    })
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing", "fkst-dev:fix-authorized" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    mock_git_push(branch)
    mock_pr_fix({ origin_marker }, branch, "feedface")

    local result = run_fix(event, opts("fix-clean-ahead-reuse", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:reviewing")
    t.eq(core.current_state({ find_raise(result.raises, "github-proxy.github_issue_comment_request").payload.body }, event.proposal_id).version, core.next_fix_version(event.version))
    t.eq(find_raise(result.raises, "devloop_reviewing").payload.version, core.next_fix_version(event.version))
    t.eq(count_calls("add -A"), 0)
    t.eq(count_calls("commit -m"), 0)
    t.eq(count_calls("git push origin"), 1)
  end,

  test_review_loop_unresolved_under_budget_reraises_review_proposal = function()
    local event = review_unresolved()
    local impl_version = reviewing().version
    local origin_marker = core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version)
    mock_bot_env()
    mock_pr_origin({ origin_marker }, "devloop-owner-repo-42-01HY", "def456")
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })
    mock_pr_diff("diff --git a/core.lua b/core.lua\n+return true\n")
    mock_pr_origin({ origin_marker }, "devloop-owner-repo-42-01HY", "def456")

    local result = run_review_loop(event, opts("review-loop-under-budget"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "consensus.proposal")
    t.is_true(result.raises[1].payload.dedup_key:find("/loop/1", 1, true) ~= nil)
    t.is_true(find_raise(result.raises, "github-proxy.github_issue_comment_request").payload.body:find("fkst:github-devloop:review-loop:v1", 1, true) ~= nil)
  end,

  test_review_loop_old_unresolved_skips_after_issue_advanced_to_newer_fixing = function()
    local review_version = reviewing().version
    local event = review_unresolved()
    local fix_version = core.fix_version_from_review_version(review_version)
    local origin_marker = core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", review_version)
    mock_bot_env()
    mock_pr_origin({ origin_marker }, "devloop-owner-repo-42-01HY", "def456")
    mock_issue_review({ "fkst-dev:fixing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "fixing", fix_version),
    })

    local result = run_review_loop(event, opts("review-loop-old-unresolved-after-fixing"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh pr diff"), 0)
  end,

	  test_review_loop_at_budget_escalates_to_review_meta_then_accepts = function()
    local event = review_unresolved({
      dedup_key = "consensus:" .. core.pr_review_proposal_id("owner/repo", 7, reviewing().version, "def456") .. "/review/loop/2",
    })
    local impl_version = reviewing().version
    local origin_marker = core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version)
    mock_bot_env()
    mock_pr_origin({ origin_marker }, "devloop-owner-repo-42-01HY", "def456")
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
      core.review_loop_marker(event.proposal_id, "github-devloop/issue/owner/repo/42", 1, "base"),
      core.review_loop_marker(event.proposal_id, "github-devloop/issue/owner/repo/42", 2, "loop1"),
    })

    local loop_result = run_review_loop(event, opts("review-loop-budget"))
    t.eq(loop_result.exit_code, 0)
    t.eq(#loop_result.raises, 3)
	    t.eq(find_raise(loop_result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:review-meta")
	    local review_meta_payload = find_raise(loop_result.raises, "devloop_review_meta").payload
    t.eq(review_meta_payload.version, impl_version .. "/review-loop/3")
    t.eq(find_raise(loop_result.raises, "github-proxy.github_issue_comment_request").payload.body:find("fkst:github-devloop:review-meta:v1", 1, true), nil)

	    mock_issue_review_meta({ "fkst-dev:review-meta" }, {
	      find_raise(loop_result.raises, "github-proxy.github_issue_comment_request").payload.body,
    })
    mock_meta_codex("accept", "The unresolved review is acceptable.")

    local meta_result = run_review_meta(review_meta_payload, opts("review-meta-accept"))
    t.eq(meta_result.exit_code, 0)
    t.eq(#meta_result.raises, 3)
	    t.eq(find_raise(meta_result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:merge-ready")
	    t.is_true(find_raise(meta_result.raises, "github-proxy.github_issue_comment_request").payload.body:find("github-devloop review-meta action: accept", 1, true) ~= nil)
    t.eq(find_raise(meta_result.raises, "devloop_merge_ready").payload.schema, "github-devloop.merge-ready.v1")
    local accept_version = core.next_review_meta_action_version(review_meta_payload.version)
    local accept_current = core.current_state({
      find_raise(meta_result.raises, "github-proxy.github_issue_comment_request").payload.body,
    }, review_meta_payload.proposal_id)
    t.eq(accept_current.state, "merge-ready")
    t.eq(accept_current.version, accept_version)
	  end,

  test_review_meta_parse_failure_errors_for_retry = function()
    local event = review_meta_event()
    mock_issue_review_meta({ "fkst-dev:review-meta" }, {
      core.state_marker(event.proposal_id, "review-meta", event.version),
    })
    t.mock_command("codex exec", {
      stdout = "unparseable review-meta answer",
      stderr = "",
      exit_code = 0,
    })

    local result = run_review_meta(event, opts("review-meta-parse-failure"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("codex exec"), 1)
  end,

  test_review_meta_fix_and_block_actions = function()
    local event = review_meta_event()
    mock_issue_review_meta({ "fkst-dev:review-meta" }, {
      core.state_marker(event.proposal_id, "review-meta", event.version),
    })
    mock_meta_codex("fix", "Run another fix pass.")
    local fix_result = run_review_meta(event, opts("review-meta-fix"))
    t.eq(fix_result.exit_code, 0)
    t.eq(#fix_result.raises, 3)
    t.eq(find_raise(fix_result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:fixing")
    t.eq(find_raise(fix_result.raises, "devloop_fixing").payload.schema, "github-devloop.fixing.v1")

    mock_issue_review_meta({ "fkst-dev:review-meta" }, {
      core.state_marker(event.proposal_id, "review-meta", event.version),
    })
    mock_meta_codex("block", "Needs human intervention.")
    local block_result = run_review_meta(event, opts("review-meta-block"))
    t.eq(block_result.exit_code, 0)
    t.eq(#block_result.raises, 2)
    t.eq(find_raise(block_result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:blocked")
  end,

  test_review_meta_marker_lag_retries_then_visible_marker_runs = function()
    local event = review_meta_event()
    mock_issue_review_meta({ "fkst-dev:enabled" }, {})

    local pending = run_review_meta(event, opts("review-meta-marker-lag"))
    t.eq(pending.exit_code, 1)
    t.eq(#pending.raises, 0)
    t.eq(count_calls("codex exec"), 0)

    mock_issue_review_meta({ "fkst-dev:review-meta" }, {
      core.state_marker(event.proposal_id, "review-meta", event.version),
    })
    mock_meta_codex("accept", "The unresolved review is acceptable.")

    local visible = run_review_meta(event, opts("review-meta-marker-visible"))
    t.eq(visible.exit_code, 0)
    t.eq(#visible.raises, 3)
    t.eq(find_raise(visible.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:merge-ready")
    t.eq(find_raise(visible.raises, "devloop_merge_ready").payload.schema, "github-devloop.merge-ready.v1")
  end,

  test_review_meta_fix_becomes_canonical_and_fix_uses_meta_feedback = function()
    local event = review_meta_event()
    local meta_exit_version = core.next_review_meta_action_version(event.version)
    mock_issue_review_meta({ "fkst-dev:review-meta" }, {
      core.state_marker(event.proposal_id, "review-meta", event.version),
    })
    mock_meta_codex("fix", "Run another fix pass.")

    local meta_result = run_review_meta(event, opts("review-meta-fix-canonical"))
    t.eq(meta_result.exit_code, 0)
    t.eq(#meta_result.raises, 3)
    local meta_comment = find_raise(meta_result.raises, "github-proxy.github_issue_comment_request").payload.body
    local current = core.current_state({
      core.state_marker(event.proposal_id, "review-meta", event.version),
      meta_comment,
    }, event.proposal_id)
    t.eq(current.state, "fixing")
    t.eq(current.version, meta_exit_version)
    local fix_event = find_raise(meta_result.raises, "devloop_fixing").payload
    t.eq(fix_event.version, meta_exit_version)

    local branch = core.implement_branch("owner/repo", "42", event.version)
    local recomputed_branch = core.implement_branch("owner/repo", "42", meta_exit_version)
    t.eq(branch ~= recomputed_branch, true)
    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", branch, event.version)
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(fix_event, { "fkst-dev:fixing", "fkst-dev:fix-authorized" }, {
      meta_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-packages-test/github-devloop/runtime", stderr = "", exit_code = 0 })
    t.mock_command("git worktree list --porcelain", {
      stdout = "worktree /tmp/fix-worktree\nHEAD def456\nbranch refs/heads/" .. branch .. "\n\n",
      stderr = "",
      exit_code = 0,
    })
    mock_implement_codex(0, "fixed review-meta feedback")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_issue_fix_for_event(fix_event, { "fkst-dev:fixing", "fkst-dev:fix-authorized" }, {
      meta_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    mock_git_push(branch)
    mock_pr_fix({ origin_marker }, branch, "feedface")

    local fix_result = run_fix(fix_event, opts("fix-from-review-meta-feedback", { FKST_GITHUB_WRITE = "1" }))
    t.eq(fix_result.exit_code, 0)
    t.eq(#fix_result.raises, 3)
    t.eq(find_raise(fix_result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:reviewing")
    t.eq(find_raise(fix_result.raises, "devloop_reviewing").payload.version, core.next_fix_version(meta_exit_version))
  end,

}
