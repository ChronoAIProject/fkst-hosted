local devloop_base = require("devloop.base")
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

local function find_raise(result, queue)
  return h.find_raise(result.raises, queue)
    or (queue:find(".", 1, true) == nil and h.find_raise(result.raises, "github-devloop-pr." .. queue) or nil)
end

local function max_fix_round_merge_ready()
  local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
  for round = 1, config.max_fix_rounds() do
    version = version .. "/fix/" .. tostring(round)
  end
  local review_proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, version, "def456")
  return h.merge_ready({
    version = version,
    review_proposal_id = review_proposal_id,
    review_dedup_key = "consensus:" .. review_proposal_id .. "/review",
  })
end

local function mock_base_head_for_merge_conflict()
  t.mock_command("git fetch origin dev", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("git rev-parse --verify 'refs/remotes/origin/dev^{commit}'", {
    stdout = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git merge-base --is-ancestor aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa def456", {
    stdout = "",
    stderr = "",
    exit_code = 1,
  })
end

local function source_comments(event, source_state)
  if source_state == "merging" then
    return h.merge_comments_with_merging(event)
  end
  return h.merge_comments(event)
end

local function run_capped_merge(source_state)
  local event = max_fix_round_merge_ready()
  local comments = source_comments(event, source_state)
  local origin_marker = m_builders.pr_origin_marker(
    event.proposal_id,
    "42",
    "devloop-owner-repo-42-01HY",
    event.version,
    "dev"
  )
  h.mock_bot_env()
  h.mock_write_env("1")
  h.mock_write_env("1")
  h.mock_issue_merge({ "fkst-dev:" .. source_state }, comments)
  h.mock_pr_merge(
    { origin_marker },
    "devloop-owner-repo-42-01HY",
    "def456",
    "OPEN",
    "owner/repo",
    false,
    "MERGEABLE",
    "DIRTY"
  )
  mock_base_head_for_merge_conflict()

  local result = h.run_merge(event, h.opts("merge-fix-cap-" .. source_state, { FKST_GITHUB_WRITE = "1" }))
  t.eq(result.exit_code, 0)
  t.eq(find_raise(result, "devloop_fixing"), nil)
  local reconcile = find_raise(result, "devloop_fix_reconcile").payload
  local decompose = find_raise(result, "github-devloop-decompose.devloop_decompose").payload
  t.eq(reconcile.issue_version, event.version)
  t.eq(reconcile.round, config.max_fix_rounds())
  t.eq(decompose.version, reconcile.issue_version)
  t.eq(decompose.head_sha, event.reviewed_head_sha)
  return reconcile, decompose
end

local function reconcile_to_blocked(source_state, reconcile)
  h.mock_bot_env()
  h.mock_default_issue_claim("owner/repo", 42)
  entity_read_mocks.mock_pr_view_selector(t, {
    comments = {
      m_builders.pr_origin_marker(
        reconcile.proposal_id,
        "42",
        "devloop-owner-repo-42-01HY",
        reconcile.issue_version,
        "dev"
      ),
      core.state_marker(reconcile.proposal_id, source_state, reconcile.issue_version),
    },
    head = "devloop-owner-repo-42-01HY",
    head_sha = reconcile.head_sha,
    base_branch = "dev",
    state = "OPEN",
  }, entity_read_mocks.pr_origin_selector, 1)
  local result = h.run_department("departments/reconcile/main.lua", {
    queue = "devloop_fix_reconcile",
    payload = reconcile,
  }, h.opts("merge-fix-cap-reconcile-" .. source_state))
  t.eq(result.exit_code, 0)
  local blocked_request = h.find_raise(result.raises, "github-proxy.github_pr_comment_request").payload
  t.is_true(blocked_request.body:find(
    core.state_marker(reconcile.proposal_id, "blocked", reconcile.issue_version),
    1,
    true
  ) ~= nil)
  t.is_true(blocked_request.body:find(
    conv_reconcile.fix_reconcile_marker(reconcile.proposal_id, reconcile.issue_version, "drop"),
    1,
    true
  ) ~= nil)
  return blocked_request.body
end

local function decompose_pr_comments(event, blocked_comment, extra)
  local comments = {
    m_builders.pr_origin_marker(
      event.proposal_id,
      "42",
      "devloop-owner-repo-42-01HY",
      event.version,
      "dev"
    ),
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
      stdout = "/tmp/fkst-packages-test/github-devloop-decompose-merge-cap/runtime",
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, 3 do
    t.mock_command("test -d", { stdout = "", stderr = "", exit_code = 1 })
  end
  t.mock_command("install -d -m 0755", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("mktemp -d", {
    stdout = "/tmp/fkst-packages-test/github-devloop-decompose-merge-cap/context/.bundle-tmp.decompose\n",
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

local function run_case(source_state)
  local reconcile, decompose = run_capped_merge(source_state)
  local blocked_comment = reconcile_to_blocked(source_state, reconcile)
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
end

return {
  test_run_graph_merge_ready_fix_cap_reconciles_blocked_then_creates_children = function()
    run_case("merge-ready")
  end,

  test_run_graph_merging_fix_cap_reconciles_blocked_then_creates_children = function()
    run_case("merging")
  end,
}
