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
  test_observe_opt_in_issue_raises_proposal_and_thinking_label = function()
    mock_issue_state({ "fkst-dev:enabled" })
    mock_issue_body("Body from GitHub")

    local result = run_observe(issue(), opts("observe-opt-in"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    t.eq(result.raises[1].queue, "consensus.proposal")
    t.eq(result.raises[1].payload.schema, "consensus.proposal.v1")
    t.eq(result.raises[1].payload.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(result.raises[1].payload.body, "Body from GitHub")
    t.eq(result.raises[1].payload.dedup_key, "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z")
    t.eq(result.raises[1].payload.source_ref.ref, "owner/repo#issue/42")

    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.schema, "github-proxy.label.v1")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:thinking")
    t.eq(label_raise.payload.issue_number, 42)
    t.eq(count_calls("gh issue view"), 2)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 1)
  end,

  test_observe_skips_not_opt_in_and_already_stateful = function()
    mock_issue_state({ "bug" })
    local not_opted = run_observe(issue({ labels = { "bug" } }), opts("observe-no-label"))
    t.eq(not_opted.exit_code, 0)
    t.eq(#not_opted.raises, 0)

    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" })
    local thinking = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:thinking" } }), opts("observe-thinking"))
    t.eq(thinking.exit_code, 0)
    t.eq(#thinking.raises, 0)
    t.eq(count_calls("gh issue view"), 2)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_re_derives_labels_and_skips_stale_enabled_payload = function()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:ready" })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled" } }), opts("observe-stale-payload"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_reconciles_regressed_label_to_canonical_marker = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:pr-open" } }), opts("observe-reconcile-reviewing"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:reviewing")
    t.eq(label_raise.payload.remove_labels[1], "fkst-dev:thinking")
    t.eq(label_raise.payload.remove_labels[3], "fkst-dev:implementing")
    t.is_true(#label_raise.payload.remove_labels >= 10)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_reraises_merge_ready_for_poll_self_heal = function()
    local event = merge_ready()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:merge-ready" }, "OPEN", merge_comments(event))

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:merge-ready" } }), opts("observe-issue-merge-ready-self-heal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local merge_raise = find_raise(result.raises, "devloop_merge_ready")
    t.eq(merge_raise.payload.schema, "github-devloop.merge-ready.v1")
    t.eq(merge_raise.payload.proposal_id, event.proposal_id)
    t.eq(merge_raise.payload.pr_number, event.pr_number)
    t.eq(merge_raise.payload.version, event.version)
    t.eq(merge_raise.payload.reviewed_head_sha, event.reviewed_head_sha)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_reraises_merging_for_poll_self_heal = function()
    local event = merge_ready()
    local comments = merge_comments(event)
    table.insert(comments, core.state_marker(event.proposal_id, "merging", event.version))
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:merging" }, "OPEN", comments)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:merging" } }), opts("observe-issue-merging-self-heal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local merge_raise = find_raise(result.raises, "devloop_merge_ready")
    t.eq(merge_raise.payload.schema, "github-devloop.merge-ready.v1")
    t.eq(merge_raise.payload.proposal_id, event.proposal_id)
    t.eq(merge_raise.payload.pr_number, event.pr_number)
    t.eq(merge_raise.payload.version, event.version)
    t.eq(merge_raise.payload.reviewed_head_sha, event.reviewed_head_sha)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_reraises_fixing_for_poll_self_heal = function()
    local event = fixing()
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      {
        proposal_id = event.review_proposal_id,
        decision = "reject",
        body = "Reject because tests failed.",
        dedup_key = event.review_dedup_key,
        source_ref = { kind = "external", ref = "owner/repo#pr/7" },
      },
      event.source_ref
    ).body
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:fixing" }, "OPEN", {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:fixing" } }), opts("observe-issue-fixing-self-heal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local fixing_raise = find_raise(result.raises, "devloop_fixing")
    t.eq(fixing_raise.payload.schema, "github-devloop.fixing.v1")
    t.eq(fixing_raise.payload.proposal_id, event.proposal_id)
    t.eq(tostring(fixing_raise.payload.pr_number), tostring(event.pr_number))
    t.eq(fixing_raise.payload.version, event.version)
    t.eq(fixing_raise.payload.review_proposal_id, event.review_proposal_id)
    t.eq(fixing_raise.payload.review_dedup_key, event.review_dedup_key)
    t.eq(fixing_raise.payload.reviewed_head_sha, event.reviewed_head_sha)
    t.eq(fixing_raise.payload.dedup_key, core.build_devloop_fixing_payload({
      proposal_id = event.proposal_id,
      impl_version = event.version,
    }, event.pr_number, {
      review_proposal_id = event.review_proposal_id,
      review_dedup_key = event.review_dedup_key,
      reviewed_head_sha = event.reviewed_head_sha,
    }, event.source_ref).dedup_key)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_skips_fixing_self_heal_after_reviewing_progress = function()
    local event = fixing()
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      {
        proposal_id = event.review_proposal_id,
        decision = "reject",
        body = "Reject because tests failed.",
        dedup_key = event.review_dedup_key,
        source_ref = { kind = "external", ref = "owner/repo#pr/7" },
      },
      event.source_ref
    ).body
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:fixing" }, "OPEN", {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
      core.state_marker(event.proposal_id, "reviewing", core.next_fix_version(event.version)),
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:fixing" } }), opts("observe-issue-fixing-self-heal-progressed"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_skips_fixing_self_heal_without_fix_fact = function()
    local event = fixing()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:fixing" }, "OPEN", {
      core.state_marker(event.proposal_id, "fixing", event.version),
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:fixing" } }), opts("observe-issue-fixing-self-heal-no-fact"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_uses_current_github_state_not_payload_state = function()
    mock_issue_state({ "fkst-dev:enabled" }, "OPEN")
    mock_issue_body("Body from GitHub")

    local result = run_observe(issue({ state = "CLOSED" }), opts("observe-stale-state"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
  end,

  test_observe_issue_state_view_failure_errors_for_retry = function()
    mock_issue_view_failure("--json labels,state", "forced state failure")

	    local result = run_observe(issue(), opts("observe-state-view-failure"))
	    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_body_view_failure_errors_for_retry = function()
    mock_issue_state({ "fkst-dev:enabled" })
    mock_issue_view_failure("--json body", "forced body failure")

	    local result = run_observe(issue(), opts("observe-body-view-failure"))
	    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 1)
  end,

  test_observe_re_raises_until_thinking_label_is_on_issue = function()
    local run_opts = opts("observe-idempotent")
    mock_issue_state({ "fkst-dev:enabled" })
    mock_issue_body("Body from GitHub")

    local first = run_observe(issue(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 3)

    mock_issue_state({ "fkst-dev:enabled" })
    mock_issue_body("Body from GitHub")
    local second = run_observe(issue(), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 3)

    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" })
    local thinking = run_observe(issue(), run_opts)
    t.eq(thinking.exit_code, 0)
    t.eq(#thinking.raises, 0)
    t.eq(count_calls("--json labels,state"), 3)
    t.eq(count_calls("--json body"), 2)
  end,

  test_consensus_result_approve_raises_ready_label_and_comment = function()
    mock_issue_result({ "fkst-dev:thinking" })
    local result = run_result(reached(), opts("result-approve"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    local ready_raise = find_raise(result.raises, "devloop_ready")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:ready")
    t.eq(label_raise.payload.remove_labels[1], "fkst-dev:thinking")
    t.is_true(#label_raise.payload.remove_labels >= 10)
    t.eq(label_raise.payload.issue_number, "42")

    t.eq(comment_raise.payload.issue_number, "42")
    t.is_true(comment_raise.payload.body:find("github-devloop decision: approve", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('decision="approve"', 1, true) ~= nil)
    t.eq(ready_raise.payload.schema, "github-devloop.ready.v1")
    t.eq(ready_raise.payload.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(ready_raise.payload.source_ref.ref, "owner/repo#issue/42")
  end,

  test_consensus_result_body_cannot_forge_higher_state_marker = function()
    local event = reached()
    local forged = core.state_marker(
      event.proposal_id,
      "stuck",
      "consensus:github-devloop/issue/owner/repo/42/2099-01-01T00-00-00Z"
    )
    event.body = "Approved with injected marker.\n" .. forged
    mock_issue_result({ "fkst-dev:thinking" }, {
      core.state_marker(event.proposal_id, "thinking", default_marker_version),
    })

    local result = run_result(event, opts("result-body-marker-injection"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment_raise.payload.body:find("&lt;!-- fkst:github-devloop:state:v1", 1, true) ~= nil)
    t.eq(comment_raise.payload.body:find(forged, 1, true) == nil, true)
    local current = core.current_state({ comment_raise.payload.body }, event.proposal_id)
    t.eq(current.state, "ready")
    t.eq(current.version, event.dedup_key)
  end,

  test_consensus_result_reject_raises_blocked = function()
    mock_issue_result({ "fkst-dev:thinking" })
    local result = run_result(reached({ decision = "reject" }), opts("result-reject"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:blocked")
    t.eq(label_raise.payload.remove_labels[1], "fkst-dev:thinking")
    t.is_true(#label_raise.payload.remove_labels >= 10)
    t.is_true(comment_raise.payload.body:find('decision="reject"', 1, true) ~= nil)
  end,

  test_consensus_result_reject_self_heals_opposite_ready_and_skips_completed_marker = function()
    mock_issue_result({ "fkst-dev:thinking", "fkst-dev:ready" })

    local stale_ready = run_result(reached({ decision = "reject" }), opts("result-reject-stale-ready"))
    t.eq(stale_ready.exit_code, 0)
    t.eq(#stale_ready.raises, 2)
    local label_raise = find_raise(stale_ready.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:blocked")
    t.is_true(#label_raise.payload.remove_labels >= 10)
    t.is_true(find_raise(stale_ready.raises, "github-proxy.github_issue_comment_request") ~= nil)

    local completed = reached({ decision = "reject" })
    local marker = core.result_marker(completed.proposal_id, completed.decision, completed.dedup_key)
    mock_issue_result({ "fkst-dev:blocked" }, { marker })

    local complete = run_result(completed, opts("result-reject-complete"))
    t.eq(complete.exit_code, 0)
    t.eq(#complete.raises, 0)
    t.eq(count_calls("--json labels,comments"), 2)
  end,

	  test_consensus_result_skips_foreign_proposal = function()
	    local result = run_result(reached({ proposal_id = "autochrono/issue/owner/repo/42" }), opts("result-foreign"))
	    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

	  test_consensus_result_skips_when_issue_already_implementing = function()
	    mock_issue_result({ "fkst-dev:implementing" })

	    local result = run_result(reached(), opts("result-implementing-terminal"))
	    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,comments"), 1)
  end,

  test_consensus_result_skips_when_issue_already_impl_failed = function()
    mock_issue_result({ "fkst-dev:impl-failed" })

    local result = run_result(reached(), opts("result-impl-failed-terminal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,comments"), 1)
  end,

  test_consensus_result_stale_approve_skips_implementing_and_stuck = function()
    mock_issue_result({ "fkst-dev:implementing" })
    local implementing = run_result(reached(), opts("result-stale-approve-implementing"))
    t.eq(implementing.exit_code, 0)
    t.eq(#implementing.raises, 0)

    mock_issue_result({ "fkst-dev:stuck" })
    local stuck_issue = run_result(reached(), opts("result-stale-approve-stuck"))
    t.eq(stuck_issue.exit_code, 0)
    t.eq(#stuck_issue.raises, 0)
  end,

  test_consensus_result_writes_marker_when_terminal_label_present_without_marker = function()
    mock_issue_result({ "fkst-dev:ready" })

	    local result = run_result(reached(), opts("result-terminal-label"))
	    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,comments"), 1)
  end,

  test_consensus_result_removes_thinking_when_terminal_label_present = function()
    mock_issue_result({ "fkst-dev:ready", "fkst-dev:thinking" })

	    local result = run_result(reached(), opts("result-terminal-plus-thinking"))
	    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_skips_stuck_when_late_reached_arrives = function()
    mock_issue_result({ "fkst-dev:stuck" })

    local result = run_result(reached(), opts("result-late-after-stuck"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_raises_label_when_result_marker_present_without_terminal_label = function()
    local current = reached()
    local marker = core.result_marker(current.proposal_id, current.decision, current.dedup_key)
    mock_issue_result({ "fkst-dev:thinking" }, { marker })

    local result = run_result(current, opts("result-marker"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:ready")
    t.is_true(find_raise(result.raises, "devloop_ready") ~= nil)
  end,

  test_consensus_result_skips_when_terminal_label_and_result_marker_present = function()
    local current = reached()
    local marker = core.result_marker(current.proposal_id, current.decision, current.dedup_key)
    mock_issue_result({ "fkst-dev:ready" }, { marker })

    local result = run_result(current, opts("result-complete"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_opposite_decision_without_thinking_skips = function()
    local current = reached({ decision = "reject" })
    local stale_marker = core.result_marker(current.proposal_id, "approve", current.dedup_key)
    mock_issue_result({ "fkst-dev:ready" }, { stale_marker })

    local result = run_result(current, opts("result-stale-opposite-marker"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_retries_when_thinking_label_is_pending = function()
    mock_issue_result({ "fkst-dev:enabled" })

	    local result = run_result(reached(), opts("result-thinking-pending"))
	    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,comments"), 1)
  end,

  test_consensus_result_older_same_direction_marker_does_not_suppress_current_version = function()
    local current = reached({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/v2",
    })
    local older_marker = core.result_marker(current.proposal_id, "approve", "consensus:github-devloop/issue/owner/repo/42/v1")
    mock_issue_result({ "fkst-dev:thinking" }, { older_marker })

    local result = run_result(current, opts("result-older-same-direction-marker"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment_raise.payload.body:find(core.result_marker(current.proposal_id, current.decision, current.dedup_key), 1, true) ~= nil)
    t.is_true(comment_raise.payload.dedup_key:find("/v2", 1, true) ~= nil)
    t.is_true(find_raise(result.raises, "devloop_ready") ~= nil)
  end,

  test_consensus_result_old_version_skips_when_newer_ready_marker_exists = function()
    local old = reached({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    })
    local newer = "consensus:github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
    mock_issue_result({ "fkst-dev:ready" }, {
      core.state_marker(old.proposal_id, "ready", newer),
    })

    local result = run_result(old, opts("result-old-version-after-new-ready"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_ignores_forged_non_bot_state_marker = function()
    local current = reached()
    mock_issue_result({ "fkst-dev:enabled" }, {
      {
        body = core.state_marker(current.proposal_id, "ready", current.dedup_key),
        author_login = "ordinary-user",
      },
    })

    local result = run_result(current, opts("result-ignore-forged-marker"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_view_failure_errors_for_retry = function()
    mock_issue_view_failure("--json labels,comments", "forced result failure")

	    local result = run_result(reached(), opts("result-view-failure"))
	    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,comments"), 1)
  end,

  test_consensus_result_rejects_malformed_proposal_id_before_gh_view = function()
    local result = run_result(reached({
      proposal_id = "github-devloop/issue/owner/repo/../../42",
      dedup_key = "github-devloop/issue/owner/repo/../../42/result",
    }), opts("result-malformed-proposal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh issue view"), 0)
  end,

  test_consensus_result_re_raises_until_github_has_terminal_fact = function()
    local run_opts = opts("result-idempotent")
    mock_issue_result({ "fkst-dev:thinking" })

    local first = run_result(reached(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 3)

    mock_issue_result({ "fkst-dev:thinking" })
    local second = run_result(reached({ body = "Different body." }), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 3)
  end,

  test_loop_unresolved_reraises_proposal_and_loop_marker_under_budget = function()
    mock_issue_loop({ "fkst-dev:thinking" })

    local result = run_loop(unresolved(), opts("loop-under-budget"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "consensus.proposal")
    t.eq(result.raises[1].payload.schema, "consensus.proposal.v1")
    t.eq(result.raises[1].payload.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(result.raises[1].payload.body, "Body from GitHub")
    t.eq(result.raises[1].payload.dedup_key, "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/1")
    t.eq(result.raises[1].payload.source_ref.ref, "owner/repo#issue/42")

    t.is_true(find_raise(result.raises, "github-proxy.github_issue_comment_request").payload.body:find(
      core.loop_marker("github-devloop/issue/owner/repo/42", 1, unresolved().dedup_key),
      1,
      true
    ) ~= nil)
    t.is_true(result.raises[2].payload.dedup_key:find("/comment/loop/1/", 1, true) ~= nil)
    t.eq(count_calls("--json title,body,updatedAt,labels,comments,state"), 1)
  end,

  test_loop_reaching_budget_raises_stuck_label_and_marker_without_proposal = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2",
    })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.loop_marker(event.proposal_id, 1, "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"),
      core.loop_marker(event.proposal_id, 2, "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/1"),
    })

    local result = run_loop(event, opts("loop-budget"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.is_true(result.raises[1].payload.body:find(core.stuck_marker(event.proposal_id, 3, event.dedup_key), 1, true) ~= nil)
    t.is_true(result.raises[1].payload.dedup_key:find("/comment/stuck/3/", 1, true) ~= nil)

    t.eq(result.raises[2].queue, "github-proxy.github_issue_label_request")
    t.eq(result.raises[2].payload.add_labels[1], "fkst-dev:stuck")
    t.eq(result.raises[2].payload.remove_labels[1], "fkst-dev:thinking")

    t.eq(result.raises[3].queue, "devloop_stuck")
    t.eq(find_raise(result.raises, "devloop_stuck").payload.schema, "github-devloop.stuck.v1")
    t.eq(find_raise(result.raises, "devloop_stuck").payload.proposal_id, event.proposal_id)
  end,

  test_loop_uses_unresolved_dedup_loop_suffix_when_github_markers_lag = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2",
    })
    mock_issue_loop({ "fkst-dev:thinking" })

    local result = run_loop(event, opts("loop-dedup-suffix-counts-marker-lag"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.is_true(result.raises[1].payload.body:find(core.stuck_marker(event.proposal_id, 3, event.dedup_key), 1, true) ~= nil)
    t.is_true(result.raises[1].payload.dedup_key:find("/comment/stuck/3/", 1, true) ~= nil)
    t.eq(result.raises[2].queue, "github-proxy.github_issue_label_request")
    t.eq(result.raises[2].payload.add_labels[1], "fkst-dev:stuck")
    t.eq(result.raises[2].payload.remove_labels[1], "fkst-dev:thinking")
    t.eq(result.raises[3].queue, "devloop_stuck")
    t.eq(find_raise(result.raises, "devloop_stuck").payload.proposal_id, event.proposal_id)
  end,

  test_loop_github_markers_ahead_of_event_still_bound_round = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/v2",
    })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.loop_marker(event.proposal_id, 1, "consensus:github-devloop/issue/owner/repo/42/base"),
      core.loop_marker(event.proposal_id, 2, "consensus:github-devloop/issue/owner/repo/42/v1"),
    })

    local result = run_loop(event, opts("loop-markers-bound-event"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.is_true(result.raises[1].payload.body:find(core.stuck_marker(event.proposal_id, 3, event.dedup_key), 1, true) ~= nil)
    t.eq(result.raises[2].queue, "github-proxy.github_issue_label_request")
    t.eq(result.raises[2].payload.add_labels[1], "fkst-dev:stuck")
    t.eq(result.raises[2].payload.remove_labels[1], "fkst-dev:thinking")
    t.eq(result.raises[3].queue, "devloop_stuck")
    t.eq(find_raise(result.raises, "devloop_stuck").payload.proposal_id, event.proposal_id)
  end,

  test_loop_skips_foreign_proposal = function()
    local result = run_loop(unresolved({ proposal_id = "autochrono/issue/owner/repo/42" }), opts("loop-foreign"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh issue view"), 0)
  end,

  test_loop_skips_already_terminal_issue = function()
    mock_issue_loop({ "fkst-dev:ready" })

    local result = run_loop(unresolved(), opts("loop-terminal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json title,body,updatedAt,labels,comments,state"), 1)
  end,

  test_loop_skips_already_implementing_issue = function()
    mock_issue_loop({ "fkst-dev:implementing" })

    local result = run_loop(unresolved(), opts("loop-implementing-terminal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json title,body,updatedAt,labels,comments,state"), 1)
  end,

  test_loop_skips_impl_failed_issue_by_label = function()
    mock_issue_loop({ "fkst-dev:impl-failed" })

    local result = run_loop(unresolved(), opts("loop-impl-failed-label"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json title,body,updatedAt,labels,comments,state"), 1)
  end,

  test_loop_retries_until_state_label_is_visible = function()
    mock_issue_loop({ "fkst-dev:enabled" })

    local pending = run_loop(unresolved(), opts("loop-state-label-pending"))
    t.eq(pending.exit_code, 1)
    t.eq(#pending.raises, 0)

    mock_issue_loop({ "fkst-dev:ready" })
    local ready = run_loop(unresolved(), opts("loop-state-label-ready"))
    t.eq(ready.exit_code, 0)
    t.eq(#ready.raises, 0)

    mock_issue_loop({ "fkst-dev:thinking" })
    local thinking = run_loop(unresolved(), opts("loop-state-label-thinking"))
    t.eq(thinking.exit_code, 0)
    t.eq(#thinking.raises, 2)
    t.eq(thinking.raises[1].queue, "consensus.proposal")
    t.eq(thinking.raises[2].queue, "github-proxy.github_issue_comment_request")
    t.eq(count_calls("--json title,body,updatedAt,labels,comments,state"), 3)
  end,

  test_loop_skips_decision_terminal_even_when_thinking_lingers = function()
    mock_issue_loop({ "fkst-dev:thinking", "fkst-dev:ready" })

    local ready = run_loop(unresolved(), opts("loop-terminal-plus-thinking"))
    t.eq(ready.exit_code, 0)
    t.eq(#ready.raises, 2)
    t.eq(count_calls("--json title,body,updatedAt,labels,comments,state"), 1)

    local stuck_event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2",
    })
    mock_issue_loop({ "fkst-dev:thinking", "fkst-dev:stuck" }, {
      core.stuck_marker(stuck_event.proposal_id, 3, stuck_event.dedup_key),
    })

    local stuck = run_loop(stuck_event, opts("loop-stuck-plus-thinking-self-heal"))
    t.eq(stuck.exit_code, 0)
    t.eq(#stuck.raises, 3)
    t.eq(find_raise(stuck.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:stuck")
    t.eq(find_raise(stuck.raises, "devloop_stuck").payload.proposal_id, stuck_event.proposal_id)
  end,

  test_loop_issue_view_failure_errors_for_retry = function()
    mock_issue_view_failure("--json title,body,updatedAt,labels,comments,state", "forced loop failure")

    local result = run_loop(unresolved(), opts("loop-view-failure"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json title,body,updatedAt,labels,comments,state"), 1)
  end,

  test_loop_duplicate_same_round_unresolved_does_not_advance_budget = function()
    local event = unresolved()
    mock_issue_loop({ "fkst-dev:thinking" }, { core.loop_marker(event.proposal_id, 1, event.dedup_key) })

    local result = run_loop(event, opts("loop-duplicate-same-round"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_loop_new_round_unresolved_advances_by_version = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/1",
    })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.loop_marker(
        event.proposal_id,
        1,
        "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
      ),
    })

    local result = run_loop(event, opts("loop-new-version-advances"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "consensus.proposal")
    t.eq(result.raises[1].payload.dedup_key, "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2")
    t.is_true(find_raise(result.raises, "github-proxy.github_issue_comment_request").payload.body:find(core.loop_marker(event.proposal_id, 2, event.dedup_key), 1, true) ~= nil)
  end,

  test_loop_duplicate_new_round_unresolved_skips_when_next_marker_exists = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/1",
    })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.loop_marker(event.proposal_id, 1, "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"),
      core.loop_marker(event.proposal_id, 2, event.dedup_key),
    })

    local result = run_loop(event, opts("loop-new-version-duplicate"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_loop_stuck_marker_idempotency_skips_repeat = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2",
    })
    mock_issue_loop({ "fkst-dev:stuck" }, { core.stuck_marker(event.proposal_id, 3, event.dedup_key) })

    local result = run_loop(event, opts("loop-idempotent-stuck-marker"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_loop_stuck_label_without_current_no_consensus_marker_errors_for_retry = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2",
    })
    mock_issue_loop({ "fkst-dev:stuck" })

    local result = run_loop(event, opts("loop-stuck-label-without-marker"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_loop_older_stuck_marker_does_not_suppress_current_version = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/v2",
    })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.loop_marker(event.proposal_id, 1, "consensus:github-devloop/issue/owner/repo/42/base"),
      core.loop_marker(event.proposal_id, 2, "consensus:github-devloop/issue/owner/repo/42/v1"),
      core.loop_marker(event.proposal_id, 3, "consensus:github-devloop/issue/owner/repo/42/v1/loop/2"),
      core.stuck_marker(event.proposal_id, 3, "consensus:github-devloop/issue/owner/repo/42/v1"),
    })

    local result = run_loop(event, opts("loop-older-stuck-marker"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.is_true(result.raises[1].payload.body:find(core.stuck_marker(event.proposal_id, 3, event.dedup_key), 1, true) ~= nil)
    t.is_true(result.raises[1].payload.dedup_key:find("/comment/stuck/3", 1, true) ~= nil)
    t.eq(result.raises[2].queue, "github-proxy.github_issue_label_request")
    t.eq(result.raises[2].payload.add_labels[1], "fkst-dev:stuck")
    t.eq(result.raises[2].payload.remove_labels[1], "fkst-dev:thinking")
    t.eq(result.raises[3].queue, "devloop_stuck")
    t.eq(find_raise(result.raises, "devloop_stuck").payload.proposal_id, event.proposal_id)
  end,

  test_loop_stuck_marker_self_heals_label_transition = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2",
    })
    mock_issue_loop({ "fkst-dev:thinking" }, { core.stuck_marker(event.proposal_id, 3, event.dedup_key) })

    local result = run_loop(event, opts("loop-stuck-marker-self-heal-label"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:stuck")
    t.eq(find_raise(result.raises, "devloop_stuck").payload.proposal_id, event.proposal_id)
  end,

}
