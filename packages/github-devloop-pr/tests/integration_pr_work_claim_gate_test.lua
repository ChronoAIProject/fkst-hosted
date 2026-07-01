local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local t = h.t
local core = h.core
local opts = h.opts
local reviewing = h.reviewing
local review_unresolved = h.review_unresolved
local review_meta_event = h.review_meta_event
local fixing = h.fixing
local review_reconcile = h.review_reconcile
local fix_reconcile = h.fix_reconcile
local mock_bot_env = h.mock_bot_env
local count_calls = h.count_calls

local function origin_marker(version)
  return m_builders.pr_origin_marker(core, 
    "github-devloop/issue/owner/repo/42",
    "42",
    "devloop-owner-repo-42-01HY",
    version,
    "dev"
  )
end

local function claim(issue_number, assignees, author_login)
  local rendered = {}
  for _, login in ipairs(assignees or {}) do
    table.insert(rendered, '{"login":"' .. h.json_string(login) .. '"}')
  end
  t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", issue_number or 42), {
    stdout = '{"assignees":[' .. table.concat(rendered, ",") .. '],"author":{"login":"' .. h.json_string(author_login or "fkst-test-bot") .. '"}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function failing_claim(issue_number)
  t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", issue_number or 42), {
    stdout = "",
    stderr = "claim read failed",
    exit_code = 1,
  })
end

local function run_review_loop_raw(payload, run_opts)
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', { stdout = "", stderr = "", exit_code = 0 })
  return t.run_department("departments/review_loop/main.lua", {
    queue = "consensus.consensus_converge",
    payload = payload,
  }, run_opts)
end

local function run_review_meta_raw(payload, run_opts)
  return t.run_department("departments/review_meta/main.lua", {
    queue = "devloop_review_meta",
    payload = payload,
  }, run_opts)
end

local function run_fix_raw(payload, run_opts)
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', { stdout = "", stderr = "", exit_code = 0 })
  return t.run_department("departments/fix/main.lua", {
    queue = "devloop_fixing",
    payload = payload,
  }, run_opts)
end

local function run_review_reconcile_raw(payload, run_opts)
  return t.run_department("departments/reconcile/main.lua", {
    queue = "devloop_review_reconcile",
    payload = payload,
  }, run_opts)
end

local function run_fix_reconcile_raw(payload, run_opts)
  return t.run_department("departments/reconcile/main.lua", {
    queue = "devloop_fix_reconcile",
    payload = payload,
  }, run_opts)
end

return {
  test_review_loop_other_owned_issue_skips_before_pr_work = function()
    local event = review_unresolved({
      round = 1,
      narrowed_question = "What should narrow?",
      angle_digests = {
        { angle = "minimal", verdict = "comment", digest = "needs more evidence" },
      },
    })
    local impl_version = reviewing().version
    mock_bot_env()
    h.mock_pr_origin_sequence({
      {
        comments = { origin_marker(impl_version) },
        head = "devloop-owner-repo-42-01HY",
        head_sha = "def456",
        base_branch = "dev",
        state = "OPEN",
      },
    })
    claim(42, { "human" })

    local result = run_review_loop_raw(event, opts("review-loop-claim-other"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh pr diff"), 0)
    t.eq(count_calls("codex exec"), 0)
  end,

  test_review_loop_claim_read_failure_fails_closed = function()
    local event = review_unresolved()
    local impl_version = reviewing().version
    mock_bot_env()
    h.mock_pr_origin_sequence({
      {
        comments = { origin_marker(impl_version) },
        head = "devloop-owner-repo-42-01HY",
        head_sha = "def456",
        base_branch = "dev",
        state = "OPEN",
      },
    })
    failing_claim(42)

    local result = run_review_loop_raw(event, opts("review-loop-claim-fails"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh pr diff"), 0)
  end,

  test_review_meta_other_owned_issue_skips_before_codex = function()
    local event = review_meta_event()
    mock_bot_env()
    claim(42, { "human" })

    local result = run_review_meta_raw(event, opts("review-meta-claim-other"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("codex exec"), 0)
  end,

  test_fix_other_owned_issue_skips_before_git_or_codex = function()
    local event = fixing()
    mock_bot_env()
    claim(42, { "human" })

    local result = run_fix_raw(event, opts("fix-claim-other", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("git fetch"), 0)
    t.eq(count_calls("codex exec"), 0)
    t.eq(count_calls("git push"), 0)
  end,

  test_fix_claim_read_failure_fails_closed_before_git_or_codex = function()
    local event = fixing()
    mock_bot_env()
    failing_claim(42)

    local result = run_fix_raw(event, opts("fix-claim-fails", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("git fetch"), 0)
    t.eq(count_calls("codex exec"), 0)
  end,

  test_review_reconcile_other_owned_issue_skips_before_pr_comment = function()
    local event = review_reconcile()
    mock_bot_env()
    claim(42, { "human" })

    local result = run_review_reconcile_raw(event, opts("review-reconcile-claim-other"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("github-proxy.github_pr_comment_request"), 0)
  end,

  test_fix_reconcile_other_owned_issue_skips_before_pr_comment = function()
    local event = fix_reconcile()
    mock_bot_env()
    claim(42, { "human" })

    local result = run_fix_reconcile_raw(event, opts("fix-reconcile-claim-other"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("github-proxy.github_pr_comment_request"), 0)
  end,

}
