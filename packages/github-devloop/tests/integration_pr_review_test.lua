local h = require("tests.devloop_helpers")
local fixtures = require("tests.production_fixture_helpers")
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
local ready = h.ready
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
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request", function(payload)
      return tostring(payload.body or ""):find("github-devloop implementation started", 1, true) ~= nil
    end)
    local open_pr_raise = find_raise(result.raises, "devloop_open_pr")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:implementing")
    t.is_true(#label_raise.payload.remove_labels >= 10)
    t.is_true(comment_raise.payload.body:find("github-devloop implementation started", 1, true) ~= nil)
    local fact = core.implementing_fact({ comment_raise.payload.body }, event.proposal_id, event.dedup_key)
    t.eq(fact.branch, branch)
    t.eq(fact.head_sha, "def456")
    t.eq(open_pr_raise.payload.schema, "github-devloop.open-pr.v1")
    t.eq(open_pr_raise.payload.proposal_id, event.proposal_id)
    t.eq(open_pr_raise.payload.version, event.dedup_key)
    t.eq(open_pr_raise.payload.branch, branch)
    t.eq(open_pr_raise.payload.head_sha, "def456")
    t.eq(open_pr_raise.payload.base_branch, "dev")

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
    t.eq(count_calls("--json title,labels,comments"), 1)
    t.eq(count_calls("git -C"), 6)
    t.eq(count_calls("git worktree add -b"), 1)
    t.eq(count_calls("codex exec"), 1)
    t.eq(count_calls("status --porcelain"), 1)
    t.eq(count_calls("add -A"), 1)
    t.eq(count_calls("commit -m"), 1)
  end,

  test_open_pr_write_raises_pr_open_request = function()
    local event = issue({ labels = { "fkst-dev:implementing" } })
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_issue_open_pr({ "fkst-dev:implementing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "implementing", impl_version),
      core.implementing_marker("github-devloop/issue/owner/repo/42", impl_version, "devloop-owner-repo-42-01HY", "abc123", "dev", "abc123"),
    })
    mock_branch_exists("devloop-owner-repo-42-01HY", "abc123")
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")

    local result = run_open_pr(event, opts("open-pr-write", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local pr_raise = find_raise(result.raises, "github-proxy.github_pr_open_request")
    t.eq(pr_raise.payload.schema, "github-proxy.pr-open.v1")
    t.eq(pr_raise.payload.branch, "devloop-owner-repo-42-01HY")
    t.eq(pr_raise.payload.head_sha, "abc123")
    t.eq(pr_raise.payload.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(pr_raise.payload.impl_version, impl_version)
    t.eq(pr_raise.payload.expected_state, "implementing")
    t.eq(pr_raise.payload.expected_version, impl_version)
    t.is_true(pr_raise.payload.body:find("fkst:github-devloop:pr-origin:v1", 1, true) ~= nil)
    t.is_true(pr_raise.payload.issue_comment_body_template:find("state=\"pr-open\"", 1, true) ~= nil)
    t.eq(pr_raise.payload.issue_label_add[1], "fkst-dev:pr-open")
    t.eq(count_calls("--json title,body,comments,labels,state"), 1)
    t.eq(count_calls("show-ref --verify --quiet"), 1)
    t.eq(count_calls("rev-parse --verify"), 1)
  end,

  test_open_pr_requires_write_switch = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_issue_open_pr({ "fkst-dev:implementing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "implementing", impl_version),
      core.implementing_marker("github-devloop/issue/owner/repo/42", impl_version, "devloop-owner-repo-42-01HY", "abc123", "dev", "abc123"),
    })
    mock_branch_exists("devloop-owner-repo-42-01HY", "abc123")
    mock_bot_env()
    mock_write_env("")
    local missing_write = run_open_pr(issue({ labels = { "fkst-dev:implementing" } }), opts("open-pr-missing-write"))
    t.eq(missing_write.exit_code, 0)
    t.eq(#missing_write.raises, 0)
  end,

  test_open_pr_write_raises_pr_open_request_without_label = function()
    local event = issue({ labels = { "fkst-dev:implementing" } })
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_issue_open_pr({ "fkst-dev:implementing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "implementing", impl_version),
      core.implementing_marker("github-devloop/issue/owner/repo/42", impl_version, "devloop-owner-repo-42-01HY", "abc123", "dev", "abc123"),
    })
    mock_branch_exists("devloop-owner-repo-42-01HY", "abc123")
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")

    local result = run_open_pr(event, opts("open-pr-write-without-label", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local pr_raise = find_raise(result.raises, "github-proxy.github_pr_open_request")
    t.eq(pr_raise.payload.schema, "github-proxy.pr-open.v1")
    t.eq(pr_raise.payload.branch, "devloop-owner-repo-42-01HY")
    t.eq(pr_raise.payload.head_sha, "abc123")
  end,

  test_observe_pr_backpointer_advances_issue_to_reviewing = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
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
    t.eq(#result.raises, 3)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    local reviewing_raise = find_raise(result.raises, "devloop_reviewing")
    t.is_true(comment_raise.payload.body:find("state=\"reviewing\"", 1, true) ~= nil)
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:reviewing")
    t.eq(reviewing_raise.payload.schema, "github-devloop.reviewing.v1")
    t.eq(reviewing_raise.payload.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(reviewing_raise.payload.pr_number, 7)
    t.eq(reviewing_raise.payload.version, impl_version)
  end,

  test_observe_pr_reconciles_regressed_label_to_reviewing_marker = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
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
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:reviewing")
    t.eq(label_raise.payload.remove_labels[1], "fkst-dev:thinking")
    t.is_true(#label_raise.payload.remove_labels >= 10)
    t.eq(count_calls("--json labels,comments"), 0)
  end,

  test_observe_pr_reraises_merge_ready_for_poll_self_heal = function()
    local event = merge_ready()
    mock_pr_origin({
      core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
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
      core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
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
    t.eq(merge_raise.payload.schema, "github-devloop.merge-ready.v1")
    t.eq(merge_raise.payload.proposal_id, event.proposal_id)
    t.eq(merge_raise.payload.pr_number, event.pr_number)
    t.eq(merge_raise.payload.version, event.version)
    t.eq(merge_raise.payload.reviewed_head_sha, event.reviewed_head_sha)
  end,

  test_observe_pr_idempotent_reviewing_marker_reraises_until_review_result_visible = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local review_id = core.pr_review_proposal_id("owner/repo", 7, impl_version, "def456")
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
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
    t.eq(#first.raises, 2)
    local reviewing_raise = find_raise(first.raises, "devloop_reviewing")
    t.eq(reviewing_raise.payload.version, impl_version)

    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_reviewing({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
      core.review_result_marker(review_id, "github-devloop/issue/owner/repo/42", "approve", "consensus:" .. review_id .. "/review"),
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
    t.eq(#reviewed.raises, 1)
  end,

  test_observe_pr_reviewing_self_heal_uses_canonical_fix_round_version = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local fix_round_version = core.next_fix_version(impl_version)
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
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
    t.eq(#result.raises, 2)
    local reviewing_raise = find_raise(result.raises, "devloop_reviewing")
    t.eq(reviewing_raise.payload.version, fix_round_version)

    mock_bot_env()
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", fix_round_version),
    })
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    }, "devloop-owner-repo-42-01HY", "feedface")
    local review = run_review_pr(reviewing_raise.payload, opts("observe-pr-reviewing-fix-round-rereview"))
    t.eq(review.exit_code, 0)
    t.eq(#review.raises, 1)
    local proposal = find_raise(review.raises, "consensus.proposal").payload
    t.eq(proposal.proposal_id, core.pr_review_proposal_id("owner/repo", 7, fix_round_version, "feedface"))
    t.is_nil(proposal.body:find("+fixed by replay", 1, true))
    t.is_true(proposal.content_fetch:find("runtime-cache:", 1, true) == 1)
  end,

  test_observe_pr_without_visible_backpointer_uses_pr_native_origin = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local branch = core.implement_branch("owner/repo", "42", impl_version)
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
    t.eq(#result.raises, 2)
    t.eq(find_raise(result.raises, "devloop_reviewing").payload.proposal_id, core.pr_proposal_id("owner/repo", 7))
    t.eq(count_calls("--json labels,comments"), 0)
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
    t.eq(#result.raises, 2)
    t.eq(find_raise(result.raises, "devloop_reviewing").payload.proposal_id, core.pr_proposal_id("owner/repo", 7))
    t.eq(count_calls("--json labels,comments"), 0)
  end,

  test_observe_pr_closed_pr_does_not_advance_issue_to_reviewing = function()
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
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
    t.eq(#result.raises, 0)
  end,

  test_observe_pr_ignores_forged_backpointer_and_uses_pr_native_origin = function()
    mock_pr_origin({
      {
        body = core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", "v1", "dev"),
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
    t.eq(#result.raises, 2)
    t.eq(find_raise(result.raises, "devloop_reviewing").payload.proposal_id, core.pr_proposal_id("owner/repo", 7))
    t.eq(count_calls("--json labels,comments"), 0)
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
    t.eq(proposal.proposal_id, core.pr_review_proposal_id("owner/repo", 7, event.version, "def456"))
    t.eq(proposal.source_ref.ref, "owner/repo#pr/7")
    t.is_true(#proposal.body < 512)
    t.is_nil(proposal.body:find("BEGIN UNTRUSTED ISSUE DATA", 1, true))
    t.is_nil(proposal.body:find("+return true", 1, true))
    t.is_true(proposal.body:find("Reviewed PR head: def456", 1, true) ~= nil)
    t.is_true(proposal.content_fetch:find("runtime-cache:", 1, true) == 1)
    t.eq(core.validate_proposal(proposal), true)
    t.eq(count_calls("--json title,labels,comments"), 1)
    t.eq(count_calls("gh pr diff"), 1)
    t.eq(count_calls("--json headRefName,headRefOid,baseRefName,state,comments"), 1)
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
    t.eq(proposal.proposal_id, core.pr_review_proposal_id("owner/repo", 7, event.version, "def456"))

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
      core.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
    })
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", event.version),
    })

    local result = run_review_result(reached_payload, opts("review-pr-gate-reject-result"))
    local fix_version = core.fix_version_from_review_version(event.version)
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    t.eq(find_raise(result.raises, "devloop_merge_ready"), nil)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local fixing_raise = find_raise(result.raises, "devloop_fixing")
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
    t.eq(count_calls("gh pr diff"), 1)
    t.eq(count_calls("--json headRefName,headRefOid,baseRefName,state,comments"), 1)
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
    })
    mock_pr_origin_sequence({
      { head = "devloop-owner-repo-42-01HY", head_sha = "def456" },
    })

    local result = run_review_pr(event, opts("review-pr-long-repo"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local proposal = result.raises[1].payload
    t.is_true(#proposal.proposal_id <= 200)
    t.eq(proposal.proposal_id, core.pr_review_proposal_id(repo, 7, version, "def456"))
    t.eq(core.validate_proposal(proposal), true)
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

  test_review_result_approve_marks_issue_merge_ready = function()
    local event = review_reached({
      angle_results = {
        { angle = "minimal", verdict = "approve" },
        { angle = "structural", verdict = "approve" },
        { angle = "delete", verdict = "approve" },
      },
    })
    local impl_version = reviewing().version
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local result = run_review_result(event, opts("review-result-approve"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    local merge_raise = find_raise(result.raises, "devloop_merge_ready")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:merge-ready")
    t.is_true(#label_raise.payload.remove_labels >= 10)
    t.is_true(comment_raise.payload.body:find("github-devloop PR review decision: approve", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find(verdict_summary_label .. "minimal=approve structural=approve delete=approve", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find(ai_sentinel, 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find("state=\"merge-ready\"", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('state="merge-ready" version="' .. impl_version .. '"', 1, true) ~= nil)
    t.eq(core.current_state({ comment_raise.payload.body }, "github-devloop/issue/owner/repo/42").version, impl_version)
    t.is_true(comment_raise.payload.body:find("fkst:github-devloop:review-result:v1", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find("fkst:github-devloop:merge-ready:v1", 1, true) ~= nil)
    t.eq(merge_raise.payload.schema, "github-devloop.merge-ready.v1")
    t.eq(tostring(merge_raise.payload.pr_number), "7")
    t.eq(merge_raise.payload.reviewed_head_sha, "def456")
  end,

  test_review_result_reject_marks_issue_fixing = function()
    local event = review_reached({ decision = "reject", body = "Review consensus rejects the diff.", blocking_gap = "missing regression guard" })
    local impl_version = reviewing().version
    local fix_version = core.fix_version_from_review_version(impl_version)
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local result = run_review_result(event, opts("review-result-reject"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    local fixing_raise = find_raise(result.raises, "devloop_fixing")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:fixing")
    t.is_true(#label_raise.payload.remove_labels >= 10)
    t.is_true(comment_raise.payload.body:find("decision=\"reject\"", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find("state=\"fixing\"", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('state="fixing" version="' .. fix_version .. '"', 1, true) ~= nil)
    t.eq(fixing_raise.payload.schema, "github-devloop.fixing.v1")
    t.eq(fixing_raise.payload.version, fix_version)
    t.eq(fixing_raise.payload.reviewed_head_sha, "def456")
  end,

  test_review_result_skips_when_pr_head_advanced_since_review = function()
    local event = review_reached()
    local impl_version = reviewing().version
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    }, "devloop-owner-repo-42-01HY", "feedface")

    local result = run_review_result(event, opts("review-result-head-advanced"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,comments"), 0)
  end,

  test_review_result_closed_pr_does_not_mark_merge_ready = function()
    local event = review_reached()
    local impl_version = reviewing().version
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    }, "devloop-owner-repo-42-01HY", "def456", "CLOSED")

    local result = run_review_result(event, opts("review-result-closed"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,comments"), 0)
  end,

  test_review_result_reject_new_fix_round_converges_over_same_review_version_merge_ready = function()
    local event = review_reached({ decision = "reject", body = "Review consensus rejects the diff.", blocking_gap = "missing regression guard" })
    local impl_version = reviewing().version
    local fix_version = core.fix_version_from_review_version(impl_version)
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:merge-ready" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "merge-ready", impl_version),
    })

    local result = run_review_result(event, opts("review-result-conflict-fixing"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:fixing")
    t.is_true(comment_raise.payload.body:find("decision=\"reject\"", 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('state="fixing" version="' .. fix_version .. '"', 1, true) ~= nil)
    local current = core.current_state({
      core.state_marker("github-devloop/issue/owner/repo/42", "merge-ready", impl_version),
      comment_raise.payload.body,
    }, "github-devloop/issue/owner/repo/42")
    t.eq(current.state, "fixing")
    t.eq(current.version, fix_version)
  end,

  test_review_result_fix_round_approve_uses_safe_review_version_consistently = function()
    local old_version = reviewing().version
    local fix_round_version = core.next_fix_version(old_version)
    local event = review_reached({
      proposal_id = core.pr_review_proposal_id("owner/repo", 7, fix_round_version, "feedface"),
      dedup_key = "consensus:" .. core.pr_review_proposal_id("owner/repo", 7, fix_round_version, "feedface") .. "/review",
    })
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", old_version, "dev"),
    }, "devloop-owner-repo-42-01HY", "feedface")
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", fix_round_version),
    })

    local result = run_review_result(event, opts("review-result-fix-round-approve"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local merge_raise = find_raise(result.raises, "devloop_merge_ready")
    t.is_true(comment_raise.payload.body:find('state="merge-ready" version="' .. fix_round_version .. '"', 1, true) ~= nil)
    t.eq(merge_raise.payload.version, fix_round_version)
    t.eq(merge_raise.payload.reviewed_head_sha, "feedface")
    local current = core.current_state({
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", fix_round_version),
      comment_raise.payload.body,
    }, "github-devloop/issue/owner/repo/42")
    t.eq(current.state, "merge-ready")
    t.eq(current.version, fix_round_version)
  end,

  test_review_result_marker_lag_retries_then_visible_marker_applies = function()
    local event = review_reached({ decision = "reject", body = "Review consensus rejects the diff.", blocking_gap = "missing regression guard" })
    local impl_version = reviewing().version
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:enabled" }, {})

    local pending = run_review_result(event, opts("review-result-marker-lag"))
    t.eq(pending.exit_code, 1)
    t.eq(#pending.raises, 0)

    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local visible = run_review_result(event, opts("review-result-marker-visible"))
    t.eq(visible.exit_code, 0)
    t.eq(#visible.raises, 3)
    t.eq(find_raise(visible.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:fixing")
  end,

  test_review_result_same_version_approve_after_reject_stale_skips = function()
    local event = review_reached()
    local impl_version = reviewing().version
    local fix_version = core.fix_version_from_review_version(impl_version)
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:fixing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "fixing", fix_version),
    })

    local result = run_review_result(event, opts("review-result-approve-after-reject"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_review_result_stale_idempotent_forged_and_foreign_skip = function()
    local event = review_reached()
    local impl_version = reviewing().version
    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:merge-ready" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "merge-ready", impl_version),
    })
    local idempotent = run_review_result(event, opts("review-result-idempotent"))
    t.eq(idempotent.exit_code, 0)
    t.eq(#idempotent.raises, 0)

    mock_pr_origin({
      core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", core.next_fix_version(impl_version)),
    })
    local stale = run_review_result(event, opts("review-result-version-mismatch"))
    t.eq(stale.exit_code, 0)
    t.eq(#stale.raises, 0)

    mock_pr_origin({
      {
        body = core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
        author_login = "ordinary-user",
      },
    }, "devloop/issue/owner/repo/v1")
    local forged = run_review_result(event, opts("review-result-forged-origin"))
    t.eq(forged.exit_code, 1)
    t.eq(#forged.raises, 0)

    local foreign = run_review_result(review_reached({
      proposal_id = "autochrono/pr-review/owner/repo/7/v1",
      dedup_key = "consensus:autochrono/pr-review/owner/repo/7/v1",
    }), opts("review-result-foreign"))
    t.eq(foreign.exit_code, 0)
    t.eq(#foreign.raises, 0)
  end,

}
