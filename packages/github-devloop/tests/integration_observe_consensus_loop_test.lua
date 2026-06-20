local h = require("tests.devloop_helpers")
require("tests.cache_seed_helpers")
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
local reconcile = h.reconcile
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
local run_result_expecting_failure = h.run_result_expecting_failure
local run_loop = h.run_loop
local run_reconcile = h.run_reconcile
local run_implement = h.run_implement
local run_observe_pr = h.run_observe_pr
local run_review_pr = h.run_review_pr
local run_review_result = h.run_review_result
local run_fix = h.run_fix
local set_pr_phase_comments = h.set_pr_phase_comments
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
local mock_issue_reconcile = h.mock_issue_reconcile
local mock_issue_implement = h.mock_issue_implement
local mock_issue_implement_raw = h.mock_issue_implement_raw
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

local function seed_cache(key, value, run_opts)
  return t.run_department("departments/test_cache_seed/main.lua", {
    queue = "cache_seed",
    payload = {
      key = key,
      value = tostring(value),
    },
  }, run_opts)
end

return {
  test_observe_opt_in_issue_raises_proposal_and_thinking_label = function()
    mock_issue_state({ "fkst-dev:enabled" })

    local result = run_observe(issue(), opts("observe-opt-in"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    t.eq(result.raises[1].queue, "consensus.proposal")
    t.eq(result.raises[1].payload.schema, "consensus.proposal.v1")
    t.eq(result.raises[1].payload.proposal_id, "github-devloop/issue/owner/repo/42")
    t.is_true(#result.raises[1].payload.body < 256)
    t.is_nil(result.raises[1].payload.body:find("Body from GitHub", 1, true))
    t.is_true(result.raises[1].payload.content_fetch:find("runtime-cache:", 1, true) == 1)
    t.is_nil(result.raises[1].payload.content_fetch:find("gh issue", 1, true))
    t.eq(result.raises[1].payload.dedup_key, "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z")
    t.eq(result.raises[1].payload.source_ref.ref, "owner/repo#issue/42")

    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.schema, "github-proxy.label.v1")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:thinking")
    t.eq(label_raise.payload.issue_number, 42)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_other_authored_unmanaged_issue_inside_grace_does_not_fork = function()
    mock_issue_state({ "fkst-dev:enabled" }, "OPEN", {}, {}, "human")
    t.mock_command(core.gh_issue_view_state_cmd("owner/repo", 42), {
      stdout = '{"title":"Issue title","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[{"name":"fkst-dev:enabled"}],"comments":[],"assignees":[],"author":{"login":"human"}}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = run_observe(issue(), opts("observe-other-author-fresh"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_create_request"), nil)
    t.eq(find_raise(result.raises, "consensus.proposal"), nil)
  end,

  test_observe_other_authored_unmanaged_issue_after_grace_raises_fork_request_only = function()
    local run_opts = opts("observe-other-author-fork")
    local seeded = seed_cache(core.fork_first_observed_key("owner/repo", 42, "2026-06-03T01:02:03Z"), now() - (3 * 60 * 60) - 1, run_opts)
    t.eq(seeded.exit_code, 0)
    mock_issue_state({ "fkst-dev:enabled" }, "OPEN", {}, {}, "human")
    t.mock_command(core.gh_issue_view_state_cmd("owner/repo", 42), {
      stdout = '{"title":"Issue title","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[{"name":"fkst-dev:enabled"}],"comments":[],"assignees":[],"author":{"login":"human"}}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = run_observe(issue(), run_opts)
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local request = find_raise(result.raises, "github-proxy.github_issue_create_request").payload
    t.eq(request.schema, "github-proxy.issue-create.v1")
    t.eq(request.assignees[1], "fkst-test-bot")
    t.eq(request.dedup_key, core.fork_issue_dedup_key("owner/repo", 42))
    t.eq(request.post_create_blocked_by.blocked_issue_number, 42)
    t.eq(find_raise(result.raises, "consensus.proposal"), nil)
  end,

  test_observe_skips_not_opt_in_and_already_stateful = function()
    mock_issue_state({ "bug" })
    local not_opted = run_observe(issue({ labels = { "bug" } }), opts("observe-no-label")) t.eq(not_opted.exit_code, 0) t.eq(#not_opted.raises, 0)
    mock_issue_state({ "fkst-class:expedite" }) local class_only = run_observe(issue({ labels = { "fkst-class:expedite" } }), opts("observe-class-label-only")) t.eq(class_only.exit_code, 0) t.eq(#class_only.raises, 0)
    mock_issue_state({ "fkst-dev:tracking" })
    local tracking = run_observe(issue({ labels = { "fkst-dev:tracking" } }), opts("observe-tracking-label")) t.eq(tracking.exit_code, 0) t.eq(#tracking.raises, 0)

    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", { { body = core.state_marker("github-devloop/issue/owner/repo/42", "thinking", "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"), created_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now()) } })
    local thinking = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:thinking" } }), opts("observe-thinking"))
    t.eq(thinking.exit_code, 0)
    t.eq(#thinking.raises, 0)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_re_derives_labels_and_skips_stale_enabled_payload = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local marker_version = "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:ready" }, "OPEN", {
      {
        id = "IC_ready_visible",
        body = core.state_marker(proposal_id, "ready", marker_version, "result-marker,ready-label,devloop-ready"),
        created_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now()),
      },
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled" } }), opts("observe-stale-payload"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local ready = find_raise(result.raises, "devloop_ready").payload
    t.eq(ready.schema, "github-devloop.ready.v1")
    t.eq(ready.ready_hand_off.comment_id, "IC_ready_visible")
    t.eq(ready.ready_hand_off.marker_version, marker_version)
    t.is_true(ready.dedup_key:find("/redrive/ready/1", 1, true) ~= nil)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_reconciles_regressed_label_to_canonical_marker = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:pr-open" } }), opts("observe-reconcile-reviewing"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.is_true(label_raise ~= nil)
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:reviewing")
    t.eq(label_raise.payload.remove_labels[1], "fkst-dev:pr-open")
    t.eq(#label_raise.payload.remove_labels, 1)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_does_not_reraise_merge_ready_after_pr_handoff = function()
    local event = merge_ready()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:merge-ready" }, "OPEN", merge_comments(event))

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:merge-ready" } }), opts("observe-issue-merge-ready-self-heal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_does_not_reraise_merging_after_pr_handoff = function()
    local event = merge_ready()
    local comments = merge_comments(event)
    table.insert(comments, core.state_marker(event.proposal_id, "merging", event.version))
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:merging" }, "OPEN", comments)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:merging" } }), opts("observe-issue-merging-self-heal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_reraises_fixing_after_restart = function()
    local event = fixing()
    local impl_version = core._strip_latest_fix_version_suffix(event.version)
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      {
        proposal_id = event.review_proposal_id,
        decision = "reject",
        body = "Reject because tests failed.",
        blocking_gap = "missing regression guard",
        dedup_key = event.review_dedup_key,
        source_ref = { kind = "external", ref = "owner/repo#pr/7" },
      },
      event.source_ref
    ).body
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:fixing" }, "OPEN", {
      core.pr_link_marker(event.proposal_id, event.pr_number, "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    })
    mock_pr_origin({
      core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      reject_comment,
    }, nil, nil, nil, nil, 2)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:fixing" } }), opts("observe-issue-fixing-self-heal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local fix_raise = find_raise(result.raises, "devloop_fixing").payload
    t.eq(fix_raise.proposal_id, event.proposal_id)
    t.eq(fix_raise.version, event.version)
    t.eq(fix_raise.review_dedup_key, event.review_dedup_key)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_skips_fixing_self_heal_after_reviewing_progress = function()
    local event = fixing()
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      core._strip_latest_fix_version_suffix(event.version),
      {
        proposal_id = event.review_proposal_id,
        decision = "reject",
        body = "Reject because tests failed.",
        blocking_gap = "missing regression guard",
        dedup_key = event.review_dedup_key,
        source_ref = { kind = "external", ref = "owner/repo#pr/7" },
      },
      event.source_ref
    ).body
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:fixing" }, "OPEN", {
      core.pr_link_marker(event.proposal_id, event.pr_number, "devloop-owner-repo-42-01HY", core._strip_latest_fix_version_suffix(event.version), "dev"),
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
      core.state_marker(event.proposal_id, "reviewing", core.next_fix_version(event.version)),
    })
    mock_pr_origin({
      core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", core._strip_latest_fix_version_suffix(event.version), "dev"),
      reject_comment,
      core.state_marker(event.proposal_id, "reviewing", core.next_fix_version(event.version)),
    }, nil, nil, nil, nil, 2)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:fixing" } }), opts("observe-issue-fixing-self-heal-progressed"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_renormalizes_fixing_without_parseable_feedback_to_reviewing = function()
    local event = fixing()
    local impl_version = core._strip_latest_fix_version_suffix(event.version)
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:fixing" }, "OPEN", {
      core.pr_link_marker(event.proposal_id, event.pr_number, "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker(event.proposal_id, "fixing", event.version),
    })
    mock_pr_origin({
      core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker(event.proposal_id, "fixing", event.version),
    }, nil, nil, nil, nil, 2)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:fixing" } }), opts("observe-issue-fixing-self-heal-no-fact"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    local reviewing_raise = find_raise(result.raises, "devloop_reviewing").payload
    t.eq(reviewing_raise.schema, "github-devloop.reviewing.v1")
    t.eq(reviewing_raise.version, core.next_fix_version(event.version))
    t.eq(reviewing_raise.pr_number, event.pr_number)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request").payload
    t.eq(label_raise.add_labels[1], "fkst-dev:reviewing")
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_reraises_review_meta_after_restart = function()
    local event = review_meta_event()
    local impl_version = reviewing().version
    local review_unresolved_event = review_unresolved({
      dedup_key = event.review_dedup_key,
      narrowed_question = "Review needs meta resolution",
      angle_digests = {
        { angle = "minimal", verdict = "comment", digest = "no fix produced" },
      },
    })
    local marker = core.review_converge_round_marker(
      event.review_proposal_id,
      event.proposal_id,
      event.version,
      "def456",
      core.source_ref_digest(event.source_ref),
      event.n - 1,
      event.review_dedup_key,
      review_unresolved_event.narrowed_question,
      review_unresolved_event.angle_digests
    )
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:review-meta" }, "OPEN", {
      core.pr_link_marker(event.proposal_id, event.pr_number, "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker(event.proposal_id, "review-meta", event.version),
      marker,
    })
    mock_pr_origin({
      core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker(event.proposal_id, "review-meta", event.version),
      marker,
    }, nil, nil, nil, nil, 2)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:review-meta" } }), opts("observe-issue-review-meta-self-heal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local meta_raise = find_raise(result.raises, "devloop_review_meta").payload
    t.eq(meta_raise.schema, "github-devloop.review-meta.v1")
    t.eq(meta_raise.proposal_id, event.proposal_id)
    t.eq(meta_raise.review_proposal_id, event.review_proposal_id)
    t.eq(meta_raise.review_dedup_key, event.review_dedup_key)
    t.eq(meta_raise.version, event.version)
    t.eq(meta_raise.pr_number, event.pr_number)
  end,

  test_observe_issue_reraises_fix_escalation_review_meta_after_restart = function()
    local fix = fixing()
    local event = review_meta_event({
      review_proposal_id = fix.review_proposal_id,
      review_dedup_key = fix.review_dedup_key,
      version = fix.version,
      n = 0,
      dedup_key = fix.dedup_key,
      source_ref = fix.source_ref,
    })
    local impl_version = reviewing().version
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:review-meta" }, "OPEN", {
      core.pr_link_marker(event.proposal_id, event.pr_number, "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker(event.proposal_id, "review-meta", event.version),
      core.review_result_marker(event.review_proposal_id, event.proposal_id, "reject", event.review_dedup_key, 1, "missing regression guard"),
    })
    set_pr_phase_comments({ "fkst-dev:review-meta" }, {
      core.state_marker(event.proposal_id, "review-meta", event.version),
      core.review_result_marker(event.review_proposal_id, event.proposal_id, "reject", event.review_dedup_key, 1, "missing regression guard"),
    })
    mock_pr_origin({ proposal_id = event.proposal_id, version = impl_version }, nil, nil, nil, nil, 2)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:review-meta" } }), opts("observe-issue-review-meta-fix-escalation"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local meta_raise = find_raise(result.raises, "devloop_review_meta").payload
    t.eq(meta_raise.schema, "github-devloop.review-meta.v1")
    t.eq(meta_raise.proposal_id, event.proposal_id)
    t.eq(meta_raise.review_proposal_id, event.review_proposal_id)
    t.eq(meta_raise.review_dedup_key, event.review_dedup_key)
    t.eq(meta_raise.version, event.version)
    t.eq(meta_raise.pr_number, event.pr_number)
    t.eq(meta_raise.n, 0)
    t.eq(meta_raise.dedup_key, core._dedup_key({
      "review-meta",
      tostring(event.proposal_id),
      tostring(event.version),
      tostring(event.pr_number),
      "0",
      tostring(event.review_dedup_key),
    }))
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_skips_review_meta_replay_with_unparseable_state_fact = function()
    local event = review_meta_event()
    local impl_version = reviewing().version
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:review-meta" }, "OPEN", {
      core.pr_link_marker(event.proposal_id, event.pr_number, "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker(event.proposal_id, "review-meta", event.version),
    })
    mock_pr_origin({
      core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      core.state_marker(event.proposal_id, "review-meta", event.version),
    }, nil, "not-a-sha", nil, nil, 2)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:review-meta" } }), opts("observe-issue-review-meta-unparseable-state-fact"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_review_meta"), nil)
    t.eq(#result.raises, 0)
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
    mock_issue_view_failure("--json title,body,comments,labels,state,updatedAt,assignees", "forced state failure")

	    local result = run_observe(issue(), opts("observe-state-view-failure"))
	    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_re_raises_until_thinking_label_is_on_issue = function()
    local run_opts = opts("observe-idempotent")
    mock_issue_state({ "fkst-dev:enabled" })

    local first = run_observe(issue(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 3)

    mock_issue_state({ "fkst-dev:enabled" })
    local second = run_observe(issue({
      updated_at = "2026-06-03T01:02:04Z",
      view_cache_key = "github-proxy/view/owner/repo/issue/42/2026-06-03T01-02-04Z",
    }), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 3)

    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", { { body = core.state_marker("github-devloop/issue/owner/repo/42", "thinking", "github-devloop/issue/owner/repo/42/2026-06-03T01-02-05Z"), created_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now()) } })
    local thinking = run_observe(issue({
      updated_at = "2026-06-03T01:02:05Z",
      view_cache_key = "github-proxy/view/owner/repo/issue/42/2026-06-03T01-02-05Z",
    }), run_opts)
    t.eq(thinking.exit_code, 0)
    local replay_proposal = find_raise(thinking.raises, "consensus.proposal").payload
    t.eq(replay_proposal.dedup_key, "github-devloop/issue/owner/repo/42/2026-06-03T01-02-05Z")
    t.eq(replay_proposal.source_ref.ref, "owner/repo#issue/42")
    t.eq(count_calls("--json body"), 0)
  end,

  test_consensus_result_approve_raises_ready_label_and_comment = function()
    mock_issue_result({ "fkst-dev:thinking" })
    local result = run_result(reached(), opts("result-approve"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:ready")
    t.eq(label_raise.payload.remove_labels[1], "fkst-dev:thinking")
    t.eq(#label_raise.payload.remove_labels, 13)
    t.eq(label_raise.payload.issue_number, "42")

    t.eq(comment_raise.payload.issue_number, "42")
    t.is_true(comment_raise.payload.body:find("github-devloop decision: approve", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('decision="approve"', 1, true) ~= nil)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
    t.eq(comment_raise.payload.handoff.kind, "github-devloop.ready")
  end,

  test_consensus_result_threads_framing_to_ready_and_implement_prompt = function()
    mock_issue_result({ "fkst-dev:thinking" })
    local result = run_result(reached({
      framing = "DO X ONLY",
    }), opts("result-approve-framing"))
    t.eq(result.exit_code, 0)
    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
    t.eq(comment_raise.payload.handoff.kind, "github-devloop.ready")

    local prompt = core.build_implement_prompt(reached().proposal_id, {
      title = "Fix parser",
      body = "Expected behavior",
    }, "DO X ONLY")
    t.is_true(prompt:find("Agreed consensus framing", 1, true) ~= nil)
    t.is_true(prompt:find("Implement EXACTLY within this", 1, true) ~= nil)
    t.is_true(prompt:find("DO X ONLY", 1, true) ~= nil)
  end,

  test_consensus_result_body_cannot_forge_higher_state_marker = function()
    local event = reached()
    local forged = core.state_marker(
      event.proposal_id,
      "blocked",
      "consensus:github-devloop/issue/owner/repo/42/2099-01-01T00-00-00Z"
    )
    event.body = "Approved with injected marker.\n" .. forged
    mock_issue_result({ "fkst-dev:thinking" }, {
      core.state_marker(event.proposal_id, "thinking", default_marker_version),
    })

    local result = run_result(event, opts("result-body-marker-injection"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment_raise.payload.body:find("&lt;!-- fkst:github-devloop:state:v1", 1, true) ~= nil)
    t.eq(comment_raise.payload.body:find(forged, 1, true) == nil, true)
    local current = core.current_state({ comment_raise.payload.body }, event.proposal_id)
    t.eq(current.state, "ready")
    t.eq(current.version, event.dedup_key)
  end,

  test_consensus_result_reject_is_unsupported = function()
    mock_issue_result({ "fkst-dev:thinking" })
    local result = run_result(reached({ decision = "reject" }), opts("result-reject"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_approve_self_heals_missing_ready_and_skips_completed_marker = function()
    mock_issue_result({ "fkst-dev:thinking", "fkst-dev:ready" })

    local stale_ready = run_result(reached(), opts("result-approve-stale-ready"))
    t.eq(stale_ready.exit_code, 0)
    t.eq(#stale_ready.raises, 2)
    local label_raise = find_raise(stale_ready.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:ready")
    t.eq(#label_raise.payload.remove_labels, 13)
    t.is_true(find_raise(stale_ready.raises, "github-proxy.github_issue_comment_request") ~= nil)
    t.eq(find_raise(stale_ready.raises, "devloop_ready"), nil)

    local completed = reached()
    local marker = core.result_marker(completed.proposal_id, completed.decision, completed.dedup_key)
    mock_issue_result({ "fkst-dev:ready" }, { marker })

    local complete = run_result(completed, opts("result-approve-complete"))
    t.eq(complete.exit_code, 0)
    t.eq(#complete.raises, 0)
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
  end,

  test_consensus_result_skips_when_issue_already_impl_failed = function()
    mock_issue_result({ "fkst-dev:impl-failed" })

    local result = run_result(reached(), opts("result-impl-failed-terminal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_stale_approve_skips_terminal_states = function()
    mock_issue_result({ "fkst-dev:implementing" })
    local implementing = run_result(reached(), opts("result-stale-approve-implementing"))
    t.eq(implementing.exit_code, 0)
    t.eq(#implementing.raises, 0)

    mock_issue_result({ "fkst-dev:blocked" })
    local blocked_issue = run_result(reached(), opts("result-stale-approve-blocked"))
    t.eq(blocked_issue.exit_code, 0)
    t.eq(#blocked_issue.raises, 0)
  end,

  test_consensus_result_writes_marker_when_terminal_label_present_without_marker = function()
    mock_issue_result({ "fkst-dev:ready" })

	    local result = run_result(reached(), opts("result-terminal-label"))
	    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_removes_thinking_when_terminal_label_present = function()
    mock_issue_result({ "fkst-dev:ready", "fkst-dev:thinking" })

	    local result = run_result(reached(), opts("result-terminal-plus-thinking"))
	    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_skips_blocked_when_late_reached_arrives = function()
    mock_issue_result({ "fkst-dev:blocked" })

    local result = run_result(reached(), opts("result-late-after-blocked"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_raises_label_when_result_marker_present_without_terminal_label = function()
    local current = reached()
    local marker = core.result_marker(current.proposal_id, current.decision, current.dedup_key)
    mock_issue_result({ "fkst-dev:thinking" }, { marker })

    local result = run_result(current, opts("result-marker"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1) t.eq(find_raise(result.raises, "github-proxy.github_issue_comment_request"), nil)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:ready")
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
  end,

  test_consensus_result_skips_when_terminal_label_and_result_marker_present = function()
    local current = reached()
    local marker = core.result_marker(current.proposal_id, current.decision, current.dedup_key)
    mock_issue_result({ "fkst-dev:ready" }, { marker })

    local result = run_result(current, opts("result-complete"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_same_decision_without_thinking_skips = function()
    local current = reached()
    local stale_marker = core.result_marker(current.proposal_id, "approve", current.dedup_key)
    mock_issue_result({ "fkst-dev:ready" }, { stale_marker })

    local result = run_result(current, opts("result-stale-same-marker"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_retries_when_thinking_label_is_pending = function()
    mock_issue_result({ "fkst-dev:enabled" })

	    local result = run_result_expecting_failure(reached(), opts("result-thinking-pending"))
	    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_older_same_direction_marker_does_not_suppress_current_version = function()
    local current = reached({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/v2",
    })
    local older_marker = core.result_marker(current.proposal_id, "approve", "consensus:github-devloop/issue/owner/repo/42/v1")
    mock_issue_result({ "fkst-dev:thinking" }, { older_marker })

    local result = run_result(current, opts("result-older-same-direction-marker"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment_raise.payload.body:find(core.result_marker(current.proposal_id, current.decision, current.dedup_key), 1, true) ~= nil)
    t.is_true(comment_raise.payload.dedup_key:find("/v2", 1, true) ~= nil)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
  end,

  test_consensus_result_uses_effect_version_for_ready_state_marker = function()
    local current = reached({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/intake/1234567890",
      effect_version = "intake/github-devloop/issue/owner/repo/42/2026-06-03T02-02-03Z",
    })
    mock_issue_result({ "fkst-dev:thinking" }, {
      core.state_marker(current.proposal_id, "thinking", current.effect_version),
    })

    local result = run_result(current, opts("result-effect-version-cas"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment_raise.payload.body:find(core.state_marker(current.proposal_id, "ready", current.effect_version, "result-marker,ready-label,devloop-ready"), 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find(core.result_marker(current.proposal_id, current.decision, current.dedup_key), 1, true) ~= nil)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
    t.eq(comment_raise.payload.handoff.marker_version, current.effect_version)
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

    local result = run_result_expecting_failure(current, opts("result-ignore-forged-marker"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_view_failure_errors_for_retry = function()
    mock_issue_view_failure("--json labels,comments", "forced result failure")

	    local result = run_result(reached(), opts("result-view-failure"))
	    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_rejects_malformed_proposal_id_before_gh_view = function()
    local result = run_result(reached({
      proposal_id = "github-devloop/issue/owner/repo/../../42",
      dedup_key = "github-devloop/issue/owner/repo/../../42/result",
    }), opts("result-malformed-proposal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_re_raises_until_github_has_terminal_fact = function()
    local run_opts = opts("result-idempotent")
    mock_issue_result({ "fkst-dev:thinking" })

    local first = run_result(reached(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 2) t.eq(find_raise(first.raises, "devloop_ready"), nil)

    mock_issue_result({ "fkst-dev:thinking" })
    local second = run_result(reached({ body = "Different body." }), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 2) t.eq(find_raise(second.raises, "devloop_ready"), nil)
  end,

  test_loop_unresolved_records_converge_round_and_reraises_proposal = function()
    mock_issue_loop({ "fkst-dev:thinking" })

    local event = unresolved({
      narrowed_question = "Can the issue be implemented as-is?",
      angle_digests = {
        { angle = "minimal", verdict = "abstain", digest = "needs-scope" },
      },
    })
    local result = run_loop(event, opts("loop-converge-round"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "consensus.proposal")
    t.eq(result.raises[1].payload.schema, "consensus.proposal.v1")
    t.eq(result.raises[1].payload.proposal_id, "github-devloop/issue/owner/repo/42")
    t.is_true(#result.raises[1].payload.body < 256)
    t.is_nil(result.raises[1].payload.body:find("Body from GitHub", 1, true))
    t.is_true(result.raises[1].payload.content_fetch:find("runtime-cache:", 1, true) == 1)
    t.is_nil(result.raises[1].payload.content_fetch:find("gh issue", 1, true))
    t.eq(result.raises[1].payload.dedup_key, "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/1")
    t.eq(result.raises[1].payload.convergence_question, event.narrowed_question)
    t.eq(result.raises[1].payload.source_ref.ref, "owner/repo#issue/42")

    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request").payload
    t.is_true(comment.body:find("fkst:github-devloop:converge-round:v1", 1, true) ~= nil)
    t.is_true(comment.body:find('round="0"', 1, true) ~= nil)
  end,

  test_loop_true_stall_records_round_and_handoff_reconcile = function()
    local base_version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local event = unresolved({
      dedup_key = base_version .. "/loop/3",
      round = 3,
      narrowed_question = "Same framing",
      angle_digests = {
        { angle = "minimal", verdict = "abstain", digest = "same" },
      },
    })
    local sr_digest = core.source_ref_digest(event.source_ref)
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.converge_round_marker(event.proposal_id, base_version, sr_digest, 1, base_version .. "/loop/1", event.narrowed_question, event.angle_digests),
      core.converge_round_marker(event.proposal_id, base_version, sr_digest, 2, base_version .. "/loop/2", event.narrowed_question, event.angle_digests),
    })

    local result = run_loop(event, opts("loop-true-stall"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.is_true(result.raises[1].payload.body:find('round="3"', 1, true) ~= nil)
    t.eq(find_raise(result.raises, "devloop_reconcile"), nil)
    t.eq(result.raises[1].payload.handoff.kind, "github-devloop.reconcile")
    t.eq(result.raises[1].payload.handoff.proposal_id, event.proposal_id)
    t.eq(result.raises[1].payload.handoff.round, 3)
    t.eq(result.raises[1].payload.handoff.base_version, base_version)
    t.eq(result.raises[1].payload.handoff.source_ref.ref, "owner/repo#issue/42")
  end,

  test_loop_round_cap_records_round_and_handoff_reconcile_even_when_question_varies = function()
    local cap = core.max_converge_rounds()
    local base_version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local function varying_digest(round)
      return {
        { angle = "minimal", verdict = "abstain", digest = "digest-" .. tostring(round) },
      }
    end
    local event = unresolved({
      dedup_key = base_version .. "/loop/" .. tostring(cap),
      round = cap,
      narrowed_question = "Question " .. tostring(cap),
      angle_digests = varying_digest(cap),
    })
    local sr_digest = core.source_ref_digest(event.source_ref)
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.converge_round_marker(event.proposal_id, base_version, sr_digest, cap - 2, base_version .. "/loop/" .. tostring(cap - 2), "Question " .. tostring(cap - 2), varying_digest(cap - 2)),
      core.converge_round_marker(event.proposal_id, base_version, sr_digest, cap - 1, base_version .. "/loop/" .. tostring(cap - 1), "Question " .. tostring(cap - 1), varying_digest(cap - 1)),
    })

    local result = run_loop(event, opts("loop-round-cap"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.is_true(result.raises[1].payload.body:find('round="' .. tostring(cap) .. '"', 1, true) ~= nil)
    t.eq(find_raise(result.raises, "devloop_reconcile"), nil)
    t.eq(result.raises[1].payload.handoff.kind, "github-devloop.reconcile")
    t.eq(result.raises[1].payload.handoff.round, cap)
    t.eq(result.raises[1].payload.handoff.base_version, base_version)
  end,

  test_loop_duplicate_converge_round_marker_skips = function()
    local event = unresolved({ round = 1 })
    local base_version = core.converge_base_version(event.dedup_key)
    local sr_digest = core.source_ref_digest(event.source_ref)
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.converge_round_marker(event.proposal_id, base_version, sr_digest, 1, event.dedup_key, nil, nil),
    })

    local result = run_loop(event, opts("loop-duplicate-converge-round"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_loop_stale_lower_round_unresolved_does_not_advance = function()
    local base_version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local event = unresolved({
      dedup_key = base_version .. "/loop/2",
      round = 2,
      narrowed_question = "Same framing",
      angle_digests = {
        { angle = "minimal", verdict = "abstain", digest = "same" },
      },
    })
    local sr_digest = core.source_ref_digest(event.source_ref)
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.converge_round_marker(event.proposal_id, base_version, sr_digest, 4, base_version .. "/loop/4", event.narrowed_question, event.angle_digests),
    })

    local result = run_loop(event, opts("loop-stale-lower-round"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_loop_skips_foreign_proposal = function()
    local result = run_loop(unresolved({ proposal_id = "autochrono/issue/owner/repo/42" }), opts("loop-foreign"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_loop_skips_already_terminal_issue = function()
    mock_issue_loop({ "fkst-dev:ready" })

    local result = run_loop(unresolved(), opts("loop-terminal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_loop_skips_already_implementing_issue = function()
    mock_issue_loop({ "fkst-dev:implementing" })

    local result = run_loop(unresolved(), opts("loop-implementing-terminal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_loop_skips_impl_failed_issue_by_label = function()
    mock_issue_loop({ "fkst-dev:impl-failed" })

    local result = run_loop(unresolved(), opts("loop-impl-failed-label"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
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
  end,

  test_loop_issue_view_failure_errors_for_retry = function()
    mock_issue_view_failure("--json title,updatedAt,labels,comments,state", "forced loop failure")

    local result = run_loop(unresolved(), opts("loop-view-failure"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_reconcile_drop_blocks_thinking_issue = function()
    local event = reconcile()
    mock_issue_reconcile({ "fkst-dev:thinking" })

    local result = run_reconcile(event, opts("reconcile-drop"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request").payload
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request").payload
    local version = core.reconcile_terminal_state_version(default_marker_version, event.round)
    t.is_true(comment.body:find("github-devloop reconcile action: drop", 1, true) ~= nil)
    t.is_true(comment.body:find("no-actionable-framing-after-3-rounds", 1, true) ~= nil)
    t.is_true(comment.body:find(core.state_marker(event.proposal_id, "blocked", version), 1, true) ~= nil)
    t.is_true(comment.body:find(core.reconcile_marker(event.proposal_id, event.base_version, event.round, "drop"), 1, true) ~= nil)
    t.eq(label.add_labels[1], "fkst-dev:blocked")
    t.eq(label.remove_labels[1], "fkst-dev:thinking")
    t.eq(count_calls("codex exec"), 0)
  end,

  test_reconcile_visible_marker_is_idempotent = function()
    local event = reconcile()
    local state_version = "github-devloop/issue/owner/repo/42/2026-06-14T05-22-55Z/intake/1287859418"
    mock_issue_reconcile({ "fkst-dev:blocked" }, {
      core.build_reconcile_comment_request("owner/repo", "42", event, "drop", "already done", core.reconcile_terminal_state_version(state_version, event.round)).body,
    })

    local result = run_reconcile(event, opts("reconcile-idempotent"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("codex exec"), 0)
  end,

  test_reconcile_version_cas_skips_newer_terminal = function()
    local event = reconcile()
    mock_issue_reconcile({ "fkst-dev:blocked" }, {
      core.state_marker(event.proposal_id, "blocked", event.base_version .. "/loop/4"),
    })

    local result = run_reconcile(event, opts("reconcile-newer-terminal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_reconcile_requires_visible_thinking_marker = function()
    mock_issue_reconcile({ "fkst-dev:enabled" })

    local result = run_reconcile(reconcile(), opts("reconcile-pending-thinking"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end
}
