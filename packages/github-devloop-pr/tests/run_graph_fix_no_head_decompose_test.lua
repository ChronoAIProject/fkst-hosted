local devloop_base = require("devloop.base")
local payloads_builders = require("devloop.payloads.builders")
local requests_review = require("devloop.requests.review")
local config = require("devloop.config")
local m_builders = require("devloop.markers.builders")
local conv_reconcile = require("devloop.convergence.reconcile")
local decompose_lib = require("devloop.decompose")
local graph = require("testkit.graph")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local author_policy = require("testkit.github_author_policy")
local h = require("tests.devloop_helpers")

local t = h.t
local core = h.core
local pr_source_ref = { kind = "external", ref = "owner/repo#pr/7" }

local function fix_version(round)
  local version = h.reviewing().version
  for _ = 1, round do
    version = core.next_fix_version(version)
  end
  return version
end

local function fixing_event(version)
  local review_version = require("contract.transition_version").safe_version_segment(
    core._strip_latest_fix_version_suffix(version)
  )
  local review_proposal_id = devloop_base.pr_review_proposal_id(
    "owner/repo",
    7,
    review_version,
    "def456"
  )
  return payloads_builders.build_devloop_fixing_payload({
    proposal_id = "github-devloop/issue/owner/repo/42",
    impl_version = version,
  }, 7, {
    review_proposal_id = review_proposal_id,
    review_dedup_key = devloop_base.pr_review_consensus_dedup_key(review_proposal_id),
    reviewed_head_sha = "def456",
    blocking_gap = "missing regression guard",
  }, pr_source_ref)
end

local function reject_comment(event)
  return requests_review.build_review_result_comment_request(core,
    "owner/repo",
    "42",
    event.proposal_id,
    event.version,
    {
      proposal_id = event.review_proposal_id,
      decision = "reject",
      body = "Review consensus rejects the diff.",
      blocking_gap = "missing regression guard",
      dedup_key = event.review_dedup_key,
      source_ref = pr_source_ref,
    },
    pr_source_ref
  ).body
end

local function run_no_new_head(event, id, post_state, feedback_comment, origin_version)
  local impl_version = origin_version or event.version
  local branch = devloop_base.implement_branch("owner/repo", "42", impl_version)
  local origin_marker = m_builders.pr_origin_marker(
    event.proposal_id,
    "42",
    branch,
    impl_version,
    "dev"
  )
  local feedback = feedback_comment or reject_comment(event)
  local comments = {
    core.state_marker(event.proposal_id, "fixing", event.version),
    feedback,
  }

  h.mock_bot_env()
  for _ = 1, 3 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_WRITE"), {
      stdout = "1",
      stderr = "",
      exit_code = 0,
    })
  end
  h.mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, comments, branch, impl_version)
  h.mock_pr_fix({ origin_marker }, branch, "def456", nil, nil, nil, 1)
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
    stderr = "",
    exit_code = 0,
  })
  h.mock_existing_fix_worktree(branch, "def456")
  h.mock_implement_codex(0, "Completed without publishing a new head.")
  h.mock_git_status(" M packages/github-devloop-pr/departments/fix/main.lua\n")
  h.mock_git_commit("def456", branch)

  local post_comments = comments
  if post_state ~= nil then
    post_comments = {
      core.state_marker(event.proposal_id, post_state.state, post_state.version),
      feedback,
    }
  end
  local post_pr_comments = { origin_marker }
  for _, comment in ipairs(post_comments) do
    table.insert(post_pr_comments, comment)
  end
  entity_read_mocks.mock_pr_view_selector(t, {
    comments = post_pr_comments,
    head = branch,
    head_sha = "def456",
    state = "OPEN",
    head_repo = "owner/repo",
    cross_repo = false,
  }, entity_read_mocks.pr_fix_selector, 1)

  return h.run_fix(event, h.opts(id, { FKST_GITHUB_WRITE = "1" }))
end

local function decompose_pr_comments(event, blocked_comment, extra)
  local comments = {
    m_builders.pr_origin_marker(event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
    blocked_comment,
  }
  for _, comment in ipairs(extra or {}) do
    table.insert(comments, comment)
  end
  return comments
end

local function mock_decompose_pr(event, comments)
  entity_read_mocks.mock_pr_view_selector(t, {
    comments = comments,
    head = "devloop-owner-repo-42-01HY",
    head_sha = event.head_sha,
    base_branch = "dev",
    state = "OPEN",
    updated_at = "2026-06-03T02:03:04Z",
  }, entity_read_mocks.pr_origin_selector, 1)
end

local function mock_decompose_execution(event, blocked_comment)
  local blocked_comments = decompose_pr_comments(event, blocked_comment)
  local decomposed_comments = decompose_pr_comments(event, blocked_comment, {
    decompose_lib.decomposed_marker(event.proposal_id, event.version, event.pr_number, 2),
  })

  author_policy.mock_env(t, {
    env = {
      FKST_DEVLOOP_MANAGED_BOT_LOGINS = "ElonSG",
      FKST_GITHUB_AUTHORIZED_LOGINS = "authorized-human",
    },
  }, {
    configure_trusted_bot_login = h.mock_author_policy_configure,
    times = 8,
  })
  for _ = 1, 6 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_WRITE"), {
      stdout = "1", stderr = "", exit_code = 0,
    })
  end
  for _ = 1, 2 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_WRITE"), {
      stdout = "", stderr = "", exit_code = 0,
    })
  end
  h.mock_default_issue_claim("owner/repo", 42)
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = "owner/repo",
    number = 42,
    title = "Original large issue",
    body = "Original body.",
    labels = { "fkst-dev:blocked" },
    comments = { blocked_comment },
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
  }, "title,body,labels,comments,author", 1)
  mock_decompose_pr(event, blocked_comments)
  mock_decompose_pr(event, blocked_comments)
  mock_decompose_pr(event, decomposed_comments)
  for _ = 1, 8 do
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop-decompose-e2e/runtime",
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, 3 do
    t.mock_command("test -d", { stdout = "", stderr = "", exit_code = 1 })
  end
  t.mock_command("install -d -m 0755", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("mktemp -d", {
    stdout = "/tmp/fkst-packages-test/github-devloop-decompose-e2e/context/.bundle-tmp.decompose\n",
    stderr = "",
    exit_code = 0,
  })
  entity_read_mocks.mock_issue_view_raw_selector(t, {}, "title,body,updatedAt,labels,comments,state,author", {
    stdout = '{"title":"Original large issue","body":"Original body.","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[{"name":"fkst-dev:blocked"}],"comments":[],"author":{"login":"fkst-test-bot"}}\n',
  })
  entity_read_mocks.mock_pr_view_raw_selector(t, {}, "title,body,headRefName,headRefOid,baseRefName,state,updatedAt,comments,labels,author", {
    stdout = '{"title":"PR title","body":"PR body","headRefName":"devloop-owner-repo-42-01HY","headRefOid":"def456","baseRefName":"dev","state":"OPEN","updatedAt":"2026-06-03T02:03:04Z","comments":[],"labels":[],"author":{"login":"fkst-test-bot"}}\n',
  })
  t.mock_command("gh pr diff", {
    stdout = "diff --git a/file.lua b/file.lua\n+return true\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh pr diff '7' --repo 'owner/repo' --name-only", {
    stdout = "file.lua\n", stderr = "", exit_code = 0,
  })
  entity_read_mocks.mock_issue_board_digest_list(t, "owner/repo", {})
  entity_read_mocks.mock_issue_list_command(t, core.gh_issue_list_recent_closed_cmd("owner/repo", 30), {})
  t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
  for _ = 1, 12 do
    t.mock_command("touch ", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("printf %s '", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command(" > ", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("test -r", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("wc -c < ", { stdout = "1\n", stderr = "", exit_code = 0 })
  end
  for _ = 1, 3 do
    t.mock_command("test -e", { stdout = "", stderr = "", exit_code = 1 })
    t.mock_command("python3 -c", { stdout = "", stderr = "", exit_code = 0 })
  end
  t.mock_command(core.gh_issue_list_decompose_children_cmd("owner/repo", event.proposal_id), {
    stdout = "[]\n", stderr = "", exit_code = 0,
  })
  t.mock_command("gh pr comment", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("codex exec", {
    stdout = [[{"issues":[{"title":"Extract a minimal retry helper","body":"Smaller scope: implement only the retry helper.\nNon-goals: do not change the whole workflow.\nAcceptance: helper tests pass."},{"title":"Wire retry helper into one call site","body":"Smaller scope: apply the helper to one path.\nNon-goals: do not rewrite unrelated states.\nAcceptance: focused integration test passes."}]}]],
    stderr = "",
    exit_code = 0,
  })
end

local function find_raise(result, queue)
  return h.find_raise(result.raises, queue)
    or (queue:find(".", 1, true) == nil and h.find_raise(result.raises, "github-devloop-pr." .. queue) or nil)
end

return {
  test_run_graph_repeated_no_new_head_reconciles_blocked_then_creates_child_issues = function()
    local below_cap = fixing_event(fix_version(config.max_fix_rounds() - 1))
    local first = run_no_new_head(below_cap, "no-new-head-below-cap")
    t.eq(first.exit_code, 0)
    local advanced = find_raise(first, "devloop_review_meta").payload
    t.eq(advanced.version, core.next_fix_version(below_cap.version))
    t.eq(core.version_fix_round(advanced.version), config.max_fix_rounds())

    h.mock_issue_review_meta({ "fkst-dev:review-meta" }, {
      core.state_marker(advanced.proposal_id, "review-meta", advanced.version),
    })
    h.mock_meta_codex("fix", "Run another fix pass.")
    local meta_result = h.run_review_meta(advanced, h.opts("no-new-head-review-meta-fix"))
    t.eq(meta_result.exit_code, 0)
    local meta_request = h.find_raise(meta_result.raises, "github-proxy.github_pr_comment_request").payload
    local meta_handoff = h.run_comment_handoff_from_request(
      meta_request,
      "IC_no_new_head_review_meta_fix_1",
      "no-new-head-review-meta-fix-handoff"
    )
    t.eq(meta_handoff.exit_code, 0)
    local at_cap = find_raise(meta_handoff, "devloop_fixing").payload
    t.eq(core.version_fix_round(at_cap.version), config.max_fix_rounds())
    t.eq(at_cap.review_dedup_key, advanced.dedup_key)

    local capped = run_no_new_head(
      at_cap,
      "no-new-head-at-cap",
      nil,
      meta_request.body,
      below_cap.version
    )
    t.eq(capped.exit_code, 0)
    t.eq(find_raise(capped, "devloop_review_meta"), nil)
    local reconcile = find_raise(capped, "devloop_fix_reconcile").payload
    local decompose = find_raise(capped, "github-devloop-decompose.devloop_decompose").payload
    t.eq(reconcile.issue_version, at_cap.version)
    t.eq(reconcile.round, config.max_fix_rounds())
    t.eq(decompose.version, reconcile.issue_version)
    t.eq(decompose.head_sha, at_cap.reviewed_head_sha)

    h.mock_bot_env()
    h.mock_default_issue_claim("owner/repo", 42)
    entity_read_mocks.mock_pr_view_selector(t, {
      comments = {
        m_builders.pr_origin_marker(reconcile.proposal_id, "42", "devloop-owner-repo-42-01HY", reconcile.issue_version, "dev"),
        core.state_marker(reconcile.proposal_id, "fixing", reconcile.issue_version),
      },
      head = "devloop-owner-repo-42-01HY",
      head_sha = reconcile.head_sha,
      base_branch = "dev",
      state = "OPEN",
    }, entity_read_mocks.pr_origin_selector, 1)
    local reconciled = h.run_department("departments/reconcile/main.lua", {
      queue = "devloop_fix_reconcile",
      payload = reconcile,
    }, h.opts("no-new-head-fix-reconcile"))
    t.eq(reconciled.exit_code, 0)
    local blocked_request = h.find_raise(reconciled.raises, "github-proxy.github_pr_comment_request").payload
    local blocked_comment = blocked_request.body
    t.is_true(blocked_comment:find(core.state_marker(reconcile.proposal_id, "blocked", reconcile.issue_version), 1, true) ~= nil)
    t.is_true(blocked_comment:find(conv_reconcile.fix_reconcile_marker(reconcile.proposal_id, reconcile.issue_version, "drop"), 1, true) ~= nil)

    mock_decompose_execution(decompose, blocked_comment)
    local trace = graph.require_quiescent(graph.run({
      queue = "github-devloop-decompose.devloop_decompose",
      payload = decompose,
      source_ref = { kind = "external", reference = "owner/repo#pr/7" },
    }, { max_steps = 6 }))
    graph.assert_covers(trace, {
      "github-devloop-decompose.devloop_decompose -> github-devloop-decompose.decompose",
      "github-proxy.github_issue_create_request -> github-proxy.github_issue_create",
    })
    local child_deliveries = 0
    for _, step in ipairs(trace.steps) do
      if step.queue == "github-proxy.github_issue_create_request"
        and step.consumer == "github-proxy.github_issue_create" then
        child_deliveries = child_deliveries + 1
      end
    end
    t.eq(child_deliveries, 2)
  end,

  test_no_new_head_drops_outcome_when_post_codex_state_is_stale = function()
    local event = fixing_event(fix_version(1))
    local result = run_no_new_head(event, "no-new-head-post-codex-stale", {
      state = "reviewing",
      version = core.next_fix_version(event.version),
    })
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,
}
