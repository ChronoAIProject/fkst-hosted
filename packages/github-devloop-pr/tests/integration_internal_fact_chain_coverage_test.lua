local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local base_ids = require("devloop.base_ids")
local requests_review = require("devloop.requests.review")
local h = require("tests.devloop_helpers")
local payloads_builders = require("devloop.payloads.builders")
local m_facts = require("devloop.markers.facts")
local t = h.t
local core = h.core
local replay_fields = require("devloop.replay_fields")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local decompose_lib = require("devloop.decompose")
local m_builders = require("devloop.markers.builders")
local opts = h.opts
local reviewing = h.reviewing
local review_reached = h.review_reached
local fixing = h.fixing
local merge_ready = h.merge_ready
local run_review_result = h.run_review_result
local run_merge = h.run_merge
local mock_issue_result = h.mock_issue_result
local mock_issue_merge = h.mock_issue_merge
local mock_pr_origin = h.mock_pr_origin
local mock_pr_origin_for = h.mock_pr_origin_for
local mock_pr_merge = h.mock_pr_merge
local mock_pr_merge_rollup = h.mock_pr_merge_rollup
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local merge_comments = h.merge_comments
local find_raise = h.find_raise
local find_causal_raise = h.find_causal_raise
local count_calls = h.count_calls

local function restart_transition_row(state_name)
  return replay_fields.restart_transition_row(core.restart_transition_table(), state_name)
end

local function mock_branch_config_env()
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
    stdout = "dev",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function find_pr_comment_with(raises, needle)
  for _, raised in ipairs(raises or {}) do
    if raised.queue == "github-proxy.github_pr_comment_request"
      and tostring((raised.payload or {}).body or ""):find(needle, 1, true) ~= nil then
      return raised
    end
  end
  return nil
end

local function find_pr_label_raise(raises)
  return find_raise(raises, "github-proxy.github_issue_label_request", function(payload)
    return tostring(payload.target_kind or "issue") == "pr"
  end)
end

local function review_origin_marker(version, head_sha)
  return m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", version, "dev")
end

local function mock_issue_result_view(labels, comments, extra)
  local fields = extra or {}
  entity_read_mocks.mock_issue_read_forms(t, {
    repo = fields.repo,
    number = fields.number,
    labels = labels,
    comments = comments,
    assignees = fields.assignees,
    author_login = fields.author_login,
    times = fields.times,
  })
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = fields.repo,
    number = fields.number,
    labels = labels,
    comments = comments,
  }, "labels,comments")
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = fields.repo,
    number = fields.number,
  }, "assignees,author")
end

local function mock_decompose_child_issue_list(event, indexes)
  local repo = base_ids.parse_proposal_id(event.proposal_id)
  local rendered = {}
  for _, index in ipairs(indexes or {}) do
    table.insert(rendered, string.format(
      '{"number":%d,"title":"Child %d","state":"OPEN","author":{"login":"fkst-test-bot"},"body":"%s","url":"https://github.example/owner/repo/issues/%d"}',
      100 + index,
      index,
      h.json_string(decompose_lib.decompose_child_marker(core, event.proposal_id, event.version, event.pr_number, index)),
      100 + index
    ))
  end
  t.mock_command(core.gh_issue_list_decompose_children_cmd(repo or "owner/repo", event.proposal_id), {
    stdout = "[" .. table.concat(rendered, ",") .. "]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function with_non_marker_comments(comments, id_prefix)
  local copied = {
    {
      id = id_prefix .. "-leading",
      author_login = "neutral-observer",
      created_at = "2026-06-12T00:00:00Z",
      body = "Neutralized production comment without fkst markers.",
    },
  }
  for _, comment in ipairs(comments or {}) do
    table.insert(copied, comment)
  end
  table.insert(copied, {
    id = id_prefix .. "-trailing",
    author_login = "neutral-observer",
    created_at = "2026-06-12T04:00:00Z",
    body = "Another neutralized production comment without replay markers.",
  })
  return copied
end

local function live_308_decompose_reconcile_marker_substream(event)
  local repo = "ChronoAIProject/fkst-packages"
  local issue_number = "285"
  local pr_number = 308
  local branch = "devloop/issue/ChronoAIProject/fkst-packages/285/ready-consensus-github-devloop-issue-ChronoAIProject-fkst-packages-285-2026-06-10T13-45-26Z"
  local proposal_id = "github-devloop/issue/ChronoAIProject/fkst-packages/285"
  local version = "ready/consensus-github-devloop/issue/ChronoAIProject/fkst-packages/285/2026-06-11T16-31-07Z/loop/1/fix/1/fix/2/fix/3/fix/4/fix/5/review-meta-action/1/review-loop/1/rereview/1/66a6dd47225a9564bed391119e2ffbf5e778ac68/fix/6/review-meta-action/2/review-loop/2/rereview/2/66a6dd47225a9564bed391119e2ffbf5e778ac68/fix/7/fix/8/fix/9/fix/10/fix/11/fix/12/fix/13/fix/14/review-loop/3/rereview/3/780d370523980b008a94aee8f028d7bccd57dbf4"
  local head_sha = "780d370523980b008a94aee8f028d7bccd57dbf4"
  event.proposal_id = proposal_id
  event.pr_number = pr_number
  event.version = version
  event.dedup_key = base_ids.dedup_key({ "decompose", proposal_id, version })
  event.source_ref = entity_lib.pr_source_ref(repo, pr_number)
  return {
    source_ref = event.source_ref,
    repo = repo,
    issue_number = issue_number,
    pr_number = pr_number,
    branch = branch,
    head_sha = head_sha,
    updated_at = "2026-06-12T01:10:51Z",
    observed_at = "2026-06-12T04:15:40Z",
    -- Runtime logs preserve the trusted marker facts but not complete comment bodies.
    -- Replay consumes bot-authored fkst markers only; non-marker stream noise is checked below.
    comments = {
      'github-devloop implementation PR for issue #285\n\n<!-- fkst:github-devloop:pr-origin:v1 proposal="github-devloop/issue/ChronoAIProject/fkst-packages/285" issue="285" branch="' .. branch .. '" impl_version="ready/consensus-github-devloop/issue/ChronoAIProject/fkst-packages/285/2026-06-11T16-31-07Z" base_branch="dev" -->',
      'github-devloop PR is ready for review\n\n<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/ChronoAIProject/fkst-packages/285" state="reviewing" version="ready/consensus-github-devloop/issue/ChronoAIProject/fkst-packages/285/2026-06-11T16-31-07Z/loop/1/fix/1/fix/2/fix/3/fix/4/fix/5/review-meta-action/1/review-loop/1/rereview/1/66a6dd47225a9564bed391119e2ffbf5e778ac68/fix/6/review-meta-action/2/review-loop/2/rereview/2/66a6dd47225a9564bed391119e2ffbf5e778ac68/fix/7/fix/8/fix/9/fix/10/fix/11/fix/12/fix/13/fix/14/review-loop/3/rereview/3" stage_rank="675" -->',
      'github-devloop fix reconcile action: drop\n\nReason:\nfix-loop-max-rounds-after-14-rounds\n\n<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/ChronoAIProject/fkst-packages/285" state="blocked" version="' .. version .. '" stage_rank="800" -->\n<!-- fkst:github-devloop:fix-reconcile:v1 proposal="github-devloop/issue/ChronoAIProject/fkst-packages/285" version="' .. version .. '" round="14" action="drop" dedup="fix-reconcile:' .. version .. '" -->\n⟦AI:FKST⟧',
      'github-devloop decomposed blocked PR into 3 follow-up issue(s)\n\n<!-- fkst:github-devloop:decomposed:v1 proposal="github-devloop/issue/ChronoAIProject/fkst-packages/285" version="' .. version .. '" pr="308" count="3" -->',
    },
  }
end

local function live_305_merge_gate_fix_marker_substream(event)
  local repo = "ChronoAIProject/fkst-packages"
  local issue_number = "300"
  local pr_number = 305
  local branch = "devloop/issue/ChronoAIProject/fkst-packages/300/ready-consensus-github-devloop-issue-ChronoAIProject-fkst-packages-300-2026-06-09T18-20-31Z"
  local proposal_id = "github-devloop/issue/ChronoAIProject/fkst-packages/300"
  local version = "ready/consensus-github-devloop/issue/ChronoAIProject/fkst-packages/300/2026-06-11T18-15-40Z/loop/1/fix/1/fix/2/fix/3/fix/4/fix/5/fix/6/fix/7/fix/8"
  local fixing_version = version .. "/fix/9"
  local head_sha = "54320f09e8f4f602b1df9a13e6bcf70998da8f1f"
  local gate_baseline_sha = "828df8d3"
  local review_proposal = "github-devloop/pr-review/ChronoAIProject-fkst-packages-2376452037/305/ready-consensus-github-devloo-0324026905/" .. head_sha
  local review_dedup = "consensus:" .. review_proposal .. "/review"
  event.proposal_id = proposal_id
  event.pr_number = pr_number
  event.version = version
  event.review_proposal_id = review_proposal
  event.review_dedup_key = review_dedup
  event.reviewed_head_sha = head_sha
  event.gate_baseline_sha = gate_baseline_sha
  event.source_ref = entity_lib.pr_source_ref(repo, pr_number)
  event.dedup_key = base_ids.dedup_key({ "merge-ready", proposal_id, version, tostring(pr_number), head_sha })
  local merge_gate = 'github-devloop merge gate failed: rollup-red\nReproduce locally with `scripts/run.sh test` from the repository root.\n\n<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/ChronoAIProject/fkst-packages/300" state="fixing" version="' .. fixing_version .. '" stage_rank="700" -->\n<!-- fkst:github-devloop:merge-gate:v1 proposal="github-devloop/issue/ChronoAIProject/fkst-packages/300" pr="305" version="' .. fixing_version .. '" review_proposal="' .. review_proposal .. '" review_dedup="' .. review_dedup .. '" head_sha="' .. head_sha .. '" gate_baseline_sha="' .. gate_baseline_sha .. '" reason="rollup-red" -->'
  return {
    repo = repo,
    issue_number = issue_number,
    pr_number = pr_number,
    branch = branch,
    head_sha = head_sha,
    updated_at = "2026-06-11T23:20:09Z",
    observed_at = "2026-06-12T04:15:39Z",
    -- Runtime logs preserve the trusted marker facts but not complete comment bodies.
    -- Replay consumes bot-authored fkst markers only; non-marker stream noise is checked below.
    pr_comments = {
      'github-devloop implementation PR for issue #300\n\n<!-- fkst:github-devloop:pr-origin:v1 proposal="github-devloop/issue/ChronoAIProject/fkst-packages/300" issue="300" branch="' .. branch .. '" impl_version="ready/consensus-github-devloop/issue/ChronoAIProject/fkst-packages/300/2026-06-11T18-15-40Z" base_branch="dev" -->',
      'github-devloop PR is ready for review\n\n<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/ChronoAIProject/fkst-packages/300" state="reviewing" version="' .. version .. '" stage_rank="675" -->',
      merge_gate,
    },
    issue_comments = {
      'github-devloop PR fix is in progress\n\n<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/ChronoAIProject/fkst-packages/300" state="fixing" version="ready/consensus-github-devloop/issue/ChronoAIProject/fkst-packages/300/2026-06-09T18-20-31Z/fix/1" stage_rank="700" -->',
      merge_gate,
    },
    fixing_version = fixing_version,
    review_proposal = review_proposal,
    review_dedup = review_dedup,
    gate_baseline_sha = gate_baseline_sha,
  }
end

local function assert_same_decompose_raise(left, right)
  t.eq(left.payload.schema, right.payload.schema)
  t.eq(left.payload.proposal_id, right.payload.proposal_id)
  t.eq(left.payload.version, right.payload.version)
  t.eq(left.payload.pr_number, right.payload.pr_number)
  t.eq(left.payload.review_proposal_id, right.payload.review_proposal_id)
  t.eq(left.payload.review_dedup_key, right.payload.review_dedup_key)
  t.eq(left.payload.head_sha, right.payload.head_sha)
  t.eq(left.payload.source_ref.ref, right.payload.source_ref.ref)
end

local function assert_same_fixing_raise(left, right)
  t.eq(left.payload.schema, right.payload.schema)
  t.eq(left.payload.proposal_id, right.payload.proposal_id)
  t.eq(left.payload.version, right.payload.version)
  t.eq(left.payload.review_proposal_id, right.payload.review_proposal_id)
  t.eq(left.payload.review_dedup_key, right.payload.review_dedup_key)
  t.eq(left.payload.reviewed_head_sha, right.payload.reviewed_head_sha)
  t.eq(left.payload.gate_baseline_sha, right.payload.gate_baseline_sha)
  t.eq(left.payload.gate_failure_excerpt, right.payload.gate_failure_excerpt)
  t.eq(left.payload.source_ref.ref, right.payload.source_ref.ref)
end

local function assert_declared_merge_gate_fixing_replay_field_set(payload)
  local row = restart_transition_row("fixing")
  t.is_true(row ~= nil)
  local expected = {}
  local expected_count = 0
  for field in pairs(row.payload_fields or {}) do
    expected[field] = true
    expected_count = expected_count + 1
  end
  t.eq(expected.review_proposal_id, true)
  t.eq(expected.review_dedup_key, true)
  t.eq(expected.reviewed_head_sha, true)
  t.eq(expected.gate_baseline_sha, true)
  t.eq(expected.source_ref, true)

  local actual_count = 0
  for field in pairs(payload or {}) do
    t.eq(expected[field], true)
    actual_count = actual_count + 1
  end
  for field in pairs(expected) do
    t.is_true(payload[field] ~= nil)
  end
  t.eq(actual_count, expected_count)
end

local function run_observe_pr_direct(run_opts)
  mock_branch_config_env()
  t.mock_command(core.gh_issue_view_claim_cmd("owner/repo", 42), {
    stdout = '{"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
    stderr = "",
    exit_code = 0,
  })
  return t.run_department("departments/observe_pr/main.lua", {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:04Z",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    },
  }, run_opts)
end

local function run_observe_pr_payload(payload, run_opts)
  mock_branch_config_env()
  return t.run_department("departments/observe_pr/main.lua", {
    queue = "github-proxy.github_entity_changed",
    payload = payload,
  }, run_opts)
end

local function reject_comment(fix)
  return requests_review.build_review_result_comment_request(core,
    "owner/repo",
    "42",
    fix.proposal_id,
    fix.version,
    {
      proposal_id = fix.review_proposal_id,
      decision = "reject",
      body = "Reject because tests failed.",
      blocking_gap = "missing regression guard",
      dedup_key = fix.review_dedup_key,
      source_ref = fix.source_ref,
    },
    fix.source_ref
  ).body
end

local function advanced_fixing_fixture(extra)
  local event = fixing()
  local branch = "devloop-owner-repo-42-01HY"
  local previous_version = event.version
  local version = core.next_fix_version(previous_version)
  local reviewed_head = "def456"
  local current_head = extra and extra.current_head or "feedface"
  local branch_head = extra and extra.branch_head or current_head
  local review_proposal = devloop_base.pr_review_proposal_id("owner/repo", 7, previous_version, reviewed_head)
  local review_dedup = "consensus:" .. review_proposal .. "/review"
  local feedback = requests_review.build_review_result_comment_request(core, "owner/repo", 42, event.proposal_id, version, {
    proposal_id = review_proposal,
    decision = "reject",
    body = "Review consensus rejects the diff.",
    blocking_gap = "missing regression guard",
    dedup_key = review_dedup,
    source_ref = event.source_ref,
  }, event.source_ref).body
  local comments = {
    m_builders.pr_origin_marker(core, event.proposal_id, "42", branch, previous_version, "dev"),
    core.state_marker(event.proposal_id, "fixing", version),
    feedback,
  }
  if extra and extra.reviewing_marker then
    table.insert(comments, core.state_marker(event.proposal_id, "reviewing", core.next_fix_version(version)))
  end
  local issue_comments = {}
  for _, comment in ipairs(comments) do
    table.insert(issue_comments, comment)
  end
  mock_bot_env()
  mock_pr_origin(comments, branch, current_head)
  mock_issue_result_view({ "fkst-dev:fixing" }, issue_comments)
  if branch_head ~= false then
    t.mock_command("git fetch origin " .. branch, {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("git rev-parse --verify 'FETCH_HEAD^{commit}'", {
      stdout = tostring(branch_head) .. "\n",
      stderr = "",
      exit_code = 0,
    })
  end
  return {
    event = event,
    version = version,
    reviewed_head = reviewed_head,
    current_head = current_head,
    branch_head = branch_head,
    review_proposal = review_proposal,
    review_dedup = review_dedup,
  }
end

return {
  test_observe_pr_fixing_head_advanced_to_branch_head_self_heals_reviewing = function()
    local fixture = advanced_fixing_fixture()
    local result = run_observe_pr_direct(opts("observe-pr-fixing-advanced-branch-head"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    local reviewing_raise = find_causal_raise(result, "devloop_reviewing")
    t.eq(reviewing_raise.payload.version, core.next_fix_version(fixture.version))
    t.eq(reviewing_raise.payload.pr_number, 7)
    t.eq(reviewing_raise.payload.source_ref.ref, "owner/repo#pr/7")
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment_raise.payload.body:find(core.state_marker(fixture.event.proposal_id, "reviewing", core.next_fix_version(fixture.version)), 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('new_head_sha="' .. fixture.current_head .. '"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('review_proposal="' .. fixture.review_proposal .. '"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('review_dedup="' .. fixture.review_dedup .. '"', 1, true) ~= nil)
    t.is_true(comment_raise.payload.body:find('review_proposal="nil"', 1, true) == nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:reviewing")
  end,

  test_observe_pr_fixing_head_advanced_reviewing_marker_idempotent_skip = function()
    local fixture = advanced_fixing_fixture({ reviewing_marker = true })
    local result = run_observe_pr_direct(opts("observe-pr-fixing-advanced-idempotent"))
    t.eq(result.exit_code, 0)
    local reviewing_raise = find_causal_raise(result, "devloop_reviewing")
    t.eq(reviewing_raise.payload.version, core.next_fix_version(fixture.version) .. "/review-loop/1")
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment_raise.payload.body:find(core.state_marker(fixture.event.proposal_id, "reviewing", reviewing_raise.payload.version), 1, true) ~= nil)
  end,

  test_observe_pr_fixing_head_advanced_not_branch_head_stays_stale = function()
    advanced_fixing_fixture({ current_head = "feedface", branch_head = "cafebabe" })
    local result = run_observe_pr_direct(opts("observe-pr-fixing-advanced-not-branch-head"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request"), nil)
  end,

  test_review_result_direct_raise_and_poll_recovery_cover_merge_ready_and_fixing = function()
    local impl_version = reviewing().version
    local approve = review_reached()
    mock_bot_env()
    mock_pr_origin({ review_origin_marker(impl_version) })
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })
    local approved = run_review_result(approve, opts("internal-chain-review-approve-direct"))
    t.eq(approved.exit_code, 0)
    local approve_comment = find_raise(approved.raises, "github-proxy.github_pr_comment_request").payload
    t.eq(find_raise(approved.raises, "devloop_merge_ready"), nil)
    t.eq(approve_comment.handoff.kind, "github-devloop.merge_ready")
    t.mock_command("gh api --method GET 'repos/owner/repo/issues/comments/IC_internal_chain_merge_ready'", {
      stdout = '{"body":"' .. h.json_string(core.state_marker(approve_comment.handoff.proposal_id, "merge-ready", approve_comment.handoff.version)) .. '","user":{"login":"fkst-test-bot"}}\n',
      stderr = "",
      exit_code = 0,
    })
    local acknowledged = t.run_department("departments/comment_handoff/main.lua", {
      queue = "github-proxy.github_comment_written",
      payload = {
        schema = "github-proxy.comment-written.v1",
        repo = approve_comment.repo,
        target = "pr",
        pr_number = approve_comment.pr_number,
        comment_id = "IC_internal_chain_merge_ready",
        request_dedup_key = approve_comment.dedup_key,
        dedup_key = tostring(approve_comment.dedup_key) .. "/written/IC_internal_chain_merge_ready",
        source_ref = approve_comment.source_ref,
        handoff = approve_comment.handoff,
      },
    }, opts("internal-chain-review-approve-handoff"))
    t.eq(acknowledged.exit_code, 0)
    local direct_merge = find_raise(acknowledged.raises, "devloop_merge_ready")
    t.eq(direct_merge.payload.schema, "github-devloop.merge-ready.v1")

    mock_pr_origin({
      review_origin_marker(impl_version),
      m_builders.review_result_marker(core, 
        direct_merge.payload.review_proposal_id,
        "github-devloop/issue/owner/repo/42",
        "approve",
        direct_merge.payload.review_dedup_key
      ),
    })
    h.set_pr_phase_comments({ "fkst-dev:merge-ready" }, merge_comments(direct_merge.payload))
    local recovered_merge = h.run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:03Z",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    }, opts("internal-chain-review-approve-recovery"))
    t.eq(recovered_merge.exit_code, 0)
    t.is_true(find_causal_raise(recovered_merge, "devloop_reviewing") ~= nil)

    local reject = review_reached({
      decision = "reject",
      body = "Review consensus rejects the diff.",
      blocking_gap = "missing regression guard",
    })
    mock_bot_env()
    mock_pr_origin({ review_origin_marker(impl_version) })
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })
    local rejected = run_review_result(reject, opts("internal-chain-review-reject-direct"))
    t.eq(rejected.exit_code, 0)
    local direct_fix = find_causal_raise(rejected, "devloop_fixing")
    t.eq(direct_fix.payload.schema, "github-devloop.fixing.v1")

    local reject_fact = find_pr_comment_with(rejected.raises, "fkst:github-devloop:review-result:v1").payload.body
    mock_pr_origin({
      review_origin_marker(impl_version),
      core.state_marker("github-devloop/issue/owner/repo/42", "fixing", direct_fix.payload.version),
      reject_fact,
    })
    mock_issue_result_view({ "fkst-dev:fixing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "fixing", direct_fix.payload.version),
      reject_fact,
    })
    local recovered_fix = h.run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:04Z",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    }, opts("internal-chain-review-reject-recovery"))
    t.eq(recovered_fix.exit_code, 0)
    t.eq(find_raise(recovered_fix.raises, "devloop_fixing"), nil)
    t.is_true(find_causal_raise(recovered_fix, "devloop_reviewing") ~= nil)
  end,

  test_merge_direct_cascade_and_poll_recovery_cover_terminal_and_repair_paths = function()
    local event = merge_ready()
    local origin_marker = review_origin_marker(event.version)
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge_rollup(merge_comments(event), '[{"__typename":"CheckRun","completedAt":"2026-06-03T02:04:04Z","conclusion":"FAILURE","detailsUrl":"https://example.invalid/checks/test","name":"test","startedAt":"2026-06-03T02:03:04Z","status":"COMPLETED","workflowName":"ci"}]')
    h.mock_required_check_runs_for(event.reviewed_head_sha, "failure")

    local red = run_merge(event, opts("internal-chain-merge-red-direct", { FKST_GITHUB_WRITE = "1" }))
    t.eq(red.exit_code, 0)
    local direct_fix = find_causal_raise(red, "devloop_fixing")
    t.eq(direct_fix.payload.schema, "github-devloop.fixing.v1")
    t.eq(direct_fix.payload.gate_baseline_sha, "abc123")
    t.eq(count_calls("gh pr merge"), 0)

    local merge_gate_comment = find_raise(red.raises, "github-proxy.github_pr_comment_request").payload.body
    t.is_true(merge_gate_comment:find('gate_baseline_sha="abc123"', 1, true) ~= nil)
    mock_pr_origin({
      origin_marker,
      core.state_marker(event.proposal_id, "fixing", direct_fix.payload.version),
      merge_gate_comment,
    })
    mock_issue_result_view({ "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", direct_fix.payload.version),
      merge_gate_comment,
    })
    local recovered_fix = h.run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = "owner/repo",
      number = 7,
      dedup_key = "owner/repo#pr#7@2026-06-04T01:02:05Z",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    }, opts("internal-chain-merge-red-recovery"))
    t.eq(recovered_fix.exit_code, 0)
    local replay_merge = find_raise(recovered_fix.raises, "devloop_merge_ready").payload
    t.eq(replay_merge.review_dedup_key, event.review_dedup_key)

    mock_bot_env()
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge(merge_comments(event), "devloop-owner-repo-42-01HY", "def456", "MERGED", "owner/repo", false, "MERGEABLE", "CLEAN", "COMPLETED", "SUCCESS", "2026-06-03T02:03:04Z")
    mock_write_env("1")
    h.mock_issue_close()
    local terminal = run_merge(event, opts("internal-chain-merge-terminal-recovery", { FKST_GITHUB_WRITE = "1" }))
    t.eq(terminal.exit_code, 0)
    t.eq(find_raise(terminal.raises, "github-proxy.github_issue_label_request"), nil)
    t.is_true(find_raise(terminal.raises, "github-proxy.github_pr_comment_request").payload.body:find("fkst:github-devloop:merged:v1", 1, true) ~= nil)
    t.eq(find_raise(terminal.raises, "devloop_fixing"), nil)
    t.eq(find_raise(terminal.raises, "devloop_reviewing"), nil)
  end,

  test_observe_pr_blocked_decomposed_marker_reraises_missing_children = function()
    local event = payloads_builders.build_devloop_decompose_payload(core, h.fix_reconcile())
    local comments = {
      m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
      core.state_marker(event.proposal_id, "blocked", event.version),
      m_builders.merge_gate_marker(core, 
        event.proposal_id,
        event.pr_number,
        event.version,
        event.review_proposal_id,
        event.review_dedup_key,
        event.head_sha,
        nil,
        "rollup-red"
      ),
      decompose_lib.decomposed_marker(core, event.proposal_id, event.version, event.pr_number, 3),
    }
    mock_bot_env()
    mock_pr_origin(comments)
    mock_issue_result_view({ "fkst-dev:blocked" }, {
      core.state_marker(event.proposal_id, "blocked", event.version),
    })
    mock_decompose_child_issue_list(event, {})

    local result = run_observe_pr_direct(opts("observe-pr-decomposed-missing-children"))

    t.eq(result.exit_code, 0)
    local decompose = find_raise(result.raises, "github-devloop-decompose.devloop_decompose")
    t.eq(decompose.payload.schema, "github-devloop.decompose.v1")
    t.eq(decompose.payload.proposal_id, event.proposal_id)
    t.eq(decompose.payload.version, event.version)
    t.eq(decompose.payload.review_proposal_id, event.review_proposal_id)
    t.eq(decompose.payload.review_dedup_key, event.review_dedup_key)
    t.eq(decompose.payload.head_sha, event.head_sha)
    t.eq(decompose.payload.source_ref.ref, "owner/repo#pr/7")
  end,

  test_observe_pr_reconciles_stale_state_label_when_expected_label_is_present = function()
    local event = payloads_builders.build_devloop_decompose_payload(core, h.fix_reconcile())
    local comments = {
      m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev"),
      core.state_marker(event.proposal_id, "blocked", event.version),
      m_builders.merge_gate_marker(core, 
        event.proposal_id,
        event.pr_number,
        event.version,
        event.review_proposal_id,
        event.review_dedup_key,
        event.head_sha,
        nil,
        "rollup-red"
      ),
      decompose_lib.decomposed_marker(core, event.proposal_id, event.version, event.pr_number, 3),
    }
    mock_bot_env()
    mock_pr_origin(comments, nil, nil, nil, nil, nil, { "fkst-dev:blocked", "fkst-dev:reviewing" })
    mock_issue_result_view({ "fkst-dev:blocked" }, {
      core.state_marker(event.proposal_id, "blocked", event.version),
    })
    mock_decompose_child_issue_list(event, { 1, 2, 3 })

    local result = run_observe_pr_direct(opts("observe-pr-reconcile-stale-reviewing-label"))

    t.eq(result.exit_code, 0)
    local label = find_pr_label_raise(result.raises).payload
    t.eq(#label.add_labels, 0)
    t.eq(#label.remove_labels, 1)
    t.eq(label.remove_labels[1], "fkst-dev:reviewing")
    t.eq(label.target_number, 7)
    t.eq(find_raise(result.raises, "github-devloop-decompose.devloop_decompose"), nil)
  end,

  test_observe_pr_live_308_decompose_reconcile_marker_substream_replay_does_not_require_fix_feedback = function()
    local event = payloads_builders.build_devloop_decompose_payload(core, h.fix_reconcile())
    event.review_proposal_id = nil
    event.review_dedup_key = nil
    event.head_sha = nil
    local fixture = live_308_decompose_reconcile_marker_substream(event)
    mock_bot_env()
    mock_pr_origin_for({
      repo = fixture.repo,
      number = fixture.pr_number,
      comments = fixture.comments,
      head = fixture.branch,
      head_sha = fixture.head_sha,
      updated_at = fixture.updated_at,
    })
    mock_issue_result_view({ "fkst-dev:blocked" }, {
      core.state_marker(event.proposal_id, "blocked", event.version),
    }, { repo = fixture.repo, number = 285 })
    mock_decompose_child_issue_list(event, {})

    local result = run_observe_pr_payload({
      schema = "github-proxy.v1",
      type = "pr",
      repo = fixture.repo,
      number = fixture.pr_number,
      dedup_key = "ChronoAIProject/fkst-packages#pr#308@2026-06-12T04:15:40Z",
      source_ref = fixture.source_ref,
    }, opts("observe-pr-live-308-decompose-reconcile-replay"))

    t.eq(result.exit_code, 0)
    local decompose = find_raise(result.raises, "github-devloop-decompose.devloop_decompose")
    t.eq(decompose.payload.schema, "github-devloop.decompose.v1")
    t.eq(decompose.payload.proposal_id, event.proposal_id)
    t.eq(decompose.payload.version, event.version)
    t.eq(decompose.payload.pr_number, event.pr_number)
    t.eq(decompose.payload.review_proposal_id, nil)
    t.eq(decompose.payload.review_dedup_key, nil)
    t.eq(decompose.payload.head_sha, nil)
    t.eq(decompose.payload.source_ref.ref, "ChronoAIProject/fkst-packages#pr/308")

    mock_bot_env()
    mock_pr_origin_for({
      repo = fixture.repo,
      number = fixture.pr_number,
      comments = with_non_marker_comments(fixture.comments, "pr-308"),
      head = fixture.branch,
      head_sha = fixture.head_sha,
      updated_at = fixture.updated_at,
    })
    mock_issue_result_view({ "fkst-dev:blocked" }, {
      core.state_marker(event.proposal_id, "blocked", event.version),
    }, { repo = fixture.repo, number = 285 })
    mock_decompose_child_issue_list(event, {})

    local noisy = run_observe_pr_payload({
      schema = "github-proxy.v1",
      type = "pr",
      repo = fixture.repo,
      number = fixture.pr_number,
      dedup_key = "ChronoAIProject/fkst-packages#pr#308@2026-06-12T04:15:40Z/noisy",
      source_ref = fixture.source_ref,
    }, opts("observe-pr-live-308-decompose-reconcile-replay-noisy"))

    t.eq(noisy.exit_code, 0)
    assert_same_decompose_raise(decompose, find_raise(noisy.raises, "github-devloop-decompose.devloop_decompose"))
  end,

  test_observe_pr_live_305_merge_gate_marker_substream_replay_reaches_issue_fixing_state = function()
    local event = merge_ready()
    local fixture = live_305_merge_gate_fix_marker_substream(event)
    mock_bot_env()
    mock_pr_origin_for({
      repo = fixture.repo,
      number = fixture.pr_number,
      comments = fixture.pr_comments,
      head = fixture.branch,
      head_sha = fixture.head_sha,
      updated_at = fixture.updated_at,
    })
    mock_issue_result_view({ "fkst-dev:fixing" }, fixture.issue_comments, { repo = fixture.repo, number = 300 })

    local result = run_observe_pr_payload({
      schema = "github-proxy.v1",
      type = "pr",
      repo = fixture.repo,
      number = fixture.pr_number,
      dedup_key = "ChronoAIProject/fkst-packages#pr#305@2026-06-12T04:15:39Z",
      source_ref = entity_lib.pr_source_ref(fixture.repo, fixture.pr_number),
    }, opts("observe-pr-live-305-merge-gate-fixing-replay"))

    t.eq(result.exit_code, 0)
    local fixing_raise = find_causal_raise(result, "devloop_fixing")
    t.eq(fixing_raise.payload.schema, "github-devloop.fixing.v1")
    t.eq(fixing_raise.payload.version, fixture.fixing_version)
    t.eq(fixing_raise.payload.review_proposal_id, fixture.review_proposal)
    t.eq(fixing_raise.payload.review_dedup_key, fixture.review_dedup)
    t.eq(fixing_raise.payload.reviewed_head_sha, event.reviewed_head_sha)
    t.eq(fixing_raise.payload.gate_baseline_sha, fixture.gate_baseline_sha)
    t.is_true(fixing_raise.payload.gate_failure_excerpt:find("rollup-red", 1, true) ~= nil)
    t.eq(fixing_raise.payload.source_ref.ref, "ChronoAIProject/fkst-packages#pr/305")
    assert_declared_merge_gate_fixing_replay_field_set(fixing_raise.payload)
    local defective_replay = payloads_builders.build_replayed_fixing_payload(core, {
      proposal_id = event.proposal_id,
      impl_version = fixture.fixing_version,
    }, fixture.pr_number, {
      review_proposal_id = fixture.review_proposal,
      review_dedup_key = fixture.review_dedup,
      reviewed_head_sha = event.reviewed_head_sha,
      blocking_gap = "rollup-red",
    }, entity_lib.pr_source_ref(fixture.repo, fixture.pr_number))
    t.is_true(defective_replay.dedup_key ~= fixing_raise.payload.dedup_key)
    t.is_true(defective_replay.dedup_key:find("/nobase/nopred/" .. tostring(event.reviewed_head_sha), 1, true) ~= nil)
    t.is_true(fixing_raise.payload.dedup_key:find("/" .. fixture.gate_baseline_sha .. "/nopred/" .. tostring(event.reviewed_head_sha), 1, true) ~= nil)
    local matching_fact = m_facts.merge_gate_fix_fact(core, fixture.pr_comments, event.proposal_id, fixture.fixing_version, {
      review_proposal_id = fixture.review_proposal,
      review_dedup_key = fixture.review_dedup,
      gate_baseline_sha = fixing_raise.payload.gate_baseline_sha,
      match_gate_baseline_sha = true,
    })
    t.eq(matching_fact.gate_baseline_sha, fixing_raise.payload.gate_baseline_sha)

    mock_bot_env()
    mock_pr_origin_for({
      repo = fixture.repo,
      number = fixture.pr_number,
      comments = with_non_marker_comments(fixture.pr_comments, "pr-305"),
      head = fixture.branch,
      head_sha = fixture.head_sha,
      updated_at = fixture.updated_at,
    })
    mock_issue_result_view({ "fkst-dev:fixing" }, with_non_marker_comments(fixture.issue_comments, "issue-300"), { repo = fixture.repo, number = 300 })

    local noisy = run_observe_pr_payload({
      schema = "github-proxy.v1",
      type = "pr",
      repo = fixture.repo,
      number = fixture.pr_number,
      dedup_key = "ChronoAIProject/fkst-packages#pr#305@2026-06-12T04:15:39Z/noisy",
      source_ref = entity_lib.pr_source_ref(fixture.repo, fixture.pr_number),
    }, opts("observe-pr-live-305-merge-gate-fixing-replay-noisy"))

    t.eq(noisy.exit_code, 0)
    assert_same_fixing_raise(fixing_raise, find_causal_raise(noisy, "devloop_fixing"))
  end,
	}
