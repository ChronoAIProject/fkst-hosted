local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local ready = h.ready
local run_implement = h.run_implement
local run_observe = h.run_observe
local issue = h.issue
local mock_issue_implement = h.mock_issue_implement
local mock_issue_state = h.mock_issue_state
local deterministic_branch_for = h.deterministic_branch_for
local mock_fresh_implement_worktree = h.mock_fresh_implement_worktree
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local count_calls = h.count_calls
local find_raise = h.find_raise

local function implementing_comments(event, extra)
  local branch = deterministic_branch_for(event)
  local comments = {
    core.state_marker(event.proposal_id, "implementing", event.dedup_key),
  }
  for _, comment in ipairs(extra or {}) do
    table.insert(comments, comment)
  end
  return comments, branch
end

local function liveness_redrive_ready(event)
  return core.build_devloop_ready_payload({
    proposal_id = event.proposal_id,
    dedup_key = event.dedup_key,
    source_ref = event.source_ref,
  })
end

local function mock_missing_remote_branch(branch)
  t.mock_command("git fetch 'origin' '" .. tostring(branch) .. "'", {
    stdout = "",
    stderr = "fatal: couldn't find remote ref",
    exit_code = 128,
  })
end

local function mock_remote_branch(branch, head)
  t.mock_command("git fetch 'origin' '" .. tostring(branch) .. "'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("refs/remotes/'origin'/'" .. tostring(branch) .. "'^{commit}", {
    stdout = tostring(head or "def456") .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_implementing_redelivery_reruns_when_no_progress_and_attempt_budget_remains = function()
    local event = ready()
    local comments, branch = implementing_comments(event, {
      core.implement_attempt_marker(event.proposal_id, event.dedup_key, 1, "1"),
    })
    mock_issue_implement({ "fkst-dev:implementing" }, comments)
    mock_missing_remote_branch(branch)
    t.mock_command("show-ref --verify --quiet", {
      stdout = "",
      stderr = "",
      exit_code = 1,
    })
    mock_fresh_implement_worktree()
    mock_implement_codex(0, "implemented after retry")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("def456", branch)
    mock_issue_implement({ "fkst-dev:implementing" }, comments)

    local result = run_implement(event, opts("implement-liveness-rerun"))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex exec"), 1)
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request").payload.body
    t.eq(core.implement_attempt_count({ comment }, event.proposal_id, event.dedup_key), 2)
    t.eq(find_raise(result.raises, "devloop_open_pr").payload.head_sha, "def456")
  end,

  test_implementing_redelivery_continues_to_open_pr_when_remote_branch_exists = function()
    local event = ready()
    local branch = deterministic_branch_for(event)
    local fact = core.implementing_marker(event.proposal_id, event.dedup_key, branch, "abc123", "dev", "abc123")
    local comments = {
      core.state_marker(event.proposal_id, "implementing", event.dedup_key),
      core.implement_attempt_marker(event.proposal_id, event.dedup_key, 1, "1"),
      fact,
    }
    mock_issue_implement({ "fkst-dev:implementing" }, comments)
    mock_remote_branch(branch, "abc123")

    local result = run_implement(event, opts("implement-liveness-remote-progress"))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex exec"), 0)
    local kickoff = find_raise(result.raises, "devloop_open_pr")
    t.eq(kickoff.payload.branch, branch)
    t.eq(kickoff.payload.head_sha, "abc123")
  end,

  test_implementing_redelivery_skips_when_pr_link_exists = function()
    local event = ready()
    local branch = deterministic_branch_for(event)
    local comments = {
      core.state_marker(event.proposal_id, "implementing", event.dedup_key),
      core.implement_attempt_marker(event.proposal_id, event.dedup_key, 1, "1"),
      core.pr_link_marker(event.proposal_id, 7, branch, event.dedup_key, "dev"),
    }
    mock_issue_implement({ "fkst-dev:implementing" }, comments)

    local result = run_implement(event, opts("implement-liveness-pr-link"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("codex exec"), 0)
  end,

  test_implementing_redelivery_marks_failed_after_attempt_budget = function()
    local event = ready()
    local comments, branch = implementing_comments(event, {
      core.implement_attempt_marker(event.proposal_id, event.dedup_key, 2, "1"),
    })
    mock_issue_implement({ "fkst-dev:implementing" }, comments)
    mock_missing_remote_branch(branch)
    t.mock_command("git fetch 'origin' 'dev'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("refs/remotes/'origin'/'dev'^{commit}", {
      stdout = "abc123\n",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("show-ref --verify --quiet", {
      stdout = "",
      stderr = "",
      exit_code = 1,
    })

    local result = run_implement(event, opts("implement-liveness-exhausted"))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex exec"), 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:impl-failed")
  end,

  test_implementing_liveness_redrive_uses_current_marker_version = function()
    local current = ready()
    local event = liveness_redrive_ready(current)
    local comments, branch = implementing_comments(current, {
      core.implement_attempt_marker(current.proposal_id, current.dedup_key, 2, "1"),
    })
    mock_issue_implement({ "fkst-dev:implementing" }, comments)
    mock_missing_remote_branch(branch)
    t.mock_command("git fetch 'origin' 'dev'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("refs/remotes/'origin'/'dev'^{commit}", {
      stdout = "abc123\n",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("show-ref --verify --quiet", {
      stdout = "",
      stderr = "",
      exit_code = 1,
    })

    local result = run_implement(event, opts("implement-liveness-redrive-current-marker"))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex exec"), 0)
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label.payload.add_labels[1], "fkst-dev:impl-failed")
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment.payload.body:find(core.state_marker(current.proposal_id, "impl-failed", current.dedup_key), 1, true) ~= nil)
  end,

  test_implementing_liveness_redrive_takes_over_orphaned_owner_marker = function()
    local current = ready()
    local event = liveness_redrive_ready(current)
    local branch = deterministic_branch_for(current)
    local comments = {
      core.state_marker(current.proposal_id, "implementing", current.dedup_key),
      core.implementing_marker(current.proposal_id, current.dedup_key, branch, "abc123", "dev", "abc123"),
    }
    mock_issue_implement({ "fkst-dev:implementing" }, comments)
    mock_missing_remote_branch(branch)
    mock_fresh_implement_worktree()
    mock_implement_codex(0, "implemented after orphan takeover")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("def456", branch)
    mock_issue_implement({ "fkst-dev:implementing" }, comments)

    local result = run_implement(event, opts("implement-liveness-redrive-orphaned-owner"))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex exec"), 1)
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request").payload.body
    t.eq(core.implement_attempt_count({ comment }, current.proposal_id, current.dedup_key), 1)
    t.eq(find_raise(result.raises, "devloop_open_pr").payload.head_sha, "def456")
  end,

  test_implementing_liveness_redrive_skips_live_attempt = function()
    local current = ready()
    local event = liveness_redrive_ready(current)
    local comments = {
      core.state_marker(current.proposal_id, "implementing", current.dedup_key),
      core.implement_attempt_marker(current.proposal_id, current.dedup_key, 1, tostring(now())),
    }
    mock_issue_implement({ "fkst-dev:implementing" }, comments)

    local result = run_implement(event, opts("implement-liveness-redrive-live-attempt"))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex exec"), 0)
    t.eq(#result.raises, 0)
  end,

  test_implementing_redelivery_recovers_local_branch_before_attempt_budget = function()
    local event = ready()
    local comments, branch = implementing_comments(event, {
      core.implement_attempt_marker(event.proposal_id, event.dedup_key, 2, "1"),
    })
    mock_issue_implement({ "fkst-dev:implementing" }, comments)
    mock_missing_remote_branch(branch)
    t.mock_command("git fetch 'origin' 'dev'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("refs/remotes/'origin'/'dev'^{commit}", {
      stdout = "abc123\n",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("show-ref --verify --quiet", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("rev-list --count", {
      stdout = "1\n",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("rev-parse --verify refs/heads/", {
      stdout = "def456\n",
      stderr = "",
      exit_code = 0,
    })

    local result = run_implement(event, opts("implement-liveness-local-progress-at-budget"))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("codex exec"), 0)
    local kickoff = find_raise(result.raises, "devloop_open_pr")
    t.eq(kickoff.payload.branch, branch)
    t.eq(kickoff.payload.head_sha, "def456")
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
  end,

  test_second_retry_death_exhausts_after_observe_reraises = function()
    local event = ready()
    local comments, branch = implementing_comments(event, {
      core.implement_attempt_marker(event.proposal_id, event.dedup_key, 2, tostring(now() - 7201)),
    })
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:implementing" }, "OPEN", comments)

    local observed = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:implementing" } }), opts("observe-implement-second-attempt-expired"))
    t.eq(observed.exit_code, 0)
    t.eq(find_raise(observed.raises, "devloop_ready").payload.proposal_id, event.proposal_id)

    mock_issue_implement({ "fkst-dev:implementing" }, comments)
    mock_missing_remote_branch(branch)
    t.mock_command("git fetch 'origin' 'dev'", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("refs/remotes/'origin'/'dev'^{commit}", {
      stdout = "abc123\n",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("show-ref --verify --quiet", {
      stdout = "",
      stderr = "",
      exit_code = 1,
    })

    local retried = run_implement(event, opts("implement-second-attempt-exhausted"))
    t.eq(retried.exit_code, 0)
    t.eq(count_calls("codex exec"), 0)
    t.eq(find_raise(retried.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:impl-failed")
  end,

  test_observe_reraises_implement_after_attempt_liveness_expires = function()
    local event = ready()
    local comments = {
      core.state_marker(event.proposal_id, "implementing", event.dedup_key),
      core.implement_attempt_marker(event.proposal_id, event.dedup_key, 1, tostring(now() - 7201)),
    }
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:implementing" }, "OPEN", comments)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:implementing" } }), opts("observe-implement-expired"))
    t.eq(result.exit_code, 0)
    local raised = find_raise(result.raises, "devloop_ready")
    t.eq(raised.payload.proposal_id, event.proposal_id)
    t.eq(raised.payload.dedup_key, "ready/" .. event.dedup_key)
  end,

  test_observe_skips_live_implement_attempt = function()
    local event = ready()
    local comments = {
      core.state_marker(event.proposal_id, "implementing", event.dedup_key),
      core.implement_attempt_marker(event.proposal_id, event.dedup_key, 1, tostring(now())),
    }
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:implementing" }, "OPEN", comments)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:implementing" } }), opts("observe-implement-live"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
  end,

  test_observe_reraises_old_implementing_marker_without_attempt_marker = function()
    local event = ready({
      dedup_key = "ready/consensus-github-devloop/issue/owner/repo/42/2026-01-01T00-00-00Z",
    })
    local branch = deterministic_branch_for(event)
    local comments = {
      core.state_marker(event.proposal_id, "implementing", event.dedup_key),
      core.implementing_marker(event.proposal_id, event.dedup_key, branch, "abc123", "dev", "abc123"),
    }
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:implementing" }, "OPEN", comments)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:implementing" } }), opts("observe-implement-old-no-attempt"))
    t.eq(result.exit_code, 0)
    local raised = find_raise(result.raises, "devloop_ready")
    t.eq(raised.payload.proposal_id, event.proposal_id)
    t.eq(raised.payload.dedup_key, "ready/" .. event.dedup_key)
  end,

  test_observe_skips_implementing_state_marker_without_progress_facts = function()
    local event = ready({
      dedup_key = "ready/consensus-github-devloop/issue/owner/repo/42/2026-01-01T00-00-00Z",
    })
    local comments = {
      core.state_marker(event.proposal_id, "implementing", event.dedup_key),
    }
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:implementing" }, "OPEN", comments)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:implementing" } }), opts("observe-implement-no-progress-facts"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
  end,

  test_observe_skips_recent_implementing_marker_without_attempt_marker = function()
    local event = ready({
      dedup_key = "ready/consensus-github-devloop/issue/owner/repo/42/2999-01-01T00-00-00Z",
    })
    local branch = deterministic_branch_for(event)
    local comments = {
      core.state_marker(event.proposal_id, "implementing", event.dedup_key),
      core.implementing_marker(event.proposal_id, event.dedup_key, branch, "abc123", "dev", "abc123"),
    }
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:implementing" }, "OPEN", comments)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:implementing" } }), opts("observe-implement-recent-no-attempt"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
  end,
}
