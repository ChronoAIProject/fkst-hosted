local devloop_base = require("devloop.base")
local requests_review = require("devloop.requests.review")
local h = require("tests.devloop_helpers")
local fixtures = require("tests.production_fixture_helpers")
local payloads_builders = require("devloop.payloads.builders")
local m_facts = require("devloop.markers.facts")
local m_builders = require("devloop.markers.builders")
local core = h.core
local t = h.t

local function review_event(extra)
  return h.review_reached(extra)
end

local function assert_valid_utf8(value)
  local ok, len = pcall(utf8.len, tostring(value or ""))
  t.is_true(ok and len ~= nil)
end

local function assert_merge_ready_handoff(result)
  local comment = h.find_raise(result.raises, "github-proxy.github_pr_comment_request")
  t.is_true(comment ~= nil)
  t.is_nil(h.find_raise(result.raises, "devloop_merge_ready"))
  t.eq(comment.payload.handoff.kind, "github-devloop.merge_ready")
  return comment.payload
end

local function assert_language_preamble(prompt)
  t.is_true(prompt:find("Write all output in English; quote code identifiers and cited originals verbatim.", 1, true) ~= nil)
end

local function assert_judge_preamble_slots(prompt)
  assert_language_preamble(prompt)
  t.is_true(prompt:find("Before judging, identify the established theory or industry best practice governing this problem class", 1, true) ~= nil)
  t.is_true(prompt:find("grounds for rejection or narrowing", 1, true) ~= nil)
  t.is_nil(prompt:find("Before acting, identify the established theory or industry best practice governing this change", 1, true))
end

local function assert_actor_preamble_slots(prompt)
  assert_language_preamble(prompt)
  t.is_true(prompt:find("Before acting, identify the established theory or industry best practice governing this change", 1, true) ~= nil)
  t.is_true(prompt:find("surface that blocker explicitly instead of silently improvising or claiming success", 1, true) ~= nil)
  t.is_nil(prompt:find("grounds for rejection or narrowing", 1, true))
end

local function assert_github_entity_history(prompt)
  t.is_true(prompt:find("Before judging, read the local context files named below.", 1, true) ~= nil)
  t.is_nil(prompt:find("gh issue view --comments / gh pr view --comments", 1, true))
end

local action_label = "⟦FKST:ACTION⟧"
local reason_label = "⟦FKST:REASON⟧"

local function meta_answer(action, reason, gap)
  local text = action_label .. " " .. action .. "\n" .. reason_label .. " " .. reason
  if gap ~= nil then
    text = text .. "\nBlocking gap: " .. gap
  end
  return text
end

return {
  test_pr_package_installs_only_pr_prompt_roles = function()
    t.eq(type(core.build_fix_prompt), "function")
    t.eq(type(core.build_review_meta_prompt), "function")
    t.eq(type(core.parse_review_meta_action), "function")
    t.is_nil(core.build_implement_prompt)
    t.is_nil(core.build_intake_prompt)
    t.is_nil(core.build_decompose_prompt)
    t.is_nil(core.build_sync_conflict_prompt)
    t.is_nil(core.parse_intake_action)
  end,

  test_pr_role_prompts_include_scoped_github_history = function()
    local issue = { title = "PR issue", comments = {} }
    local manifest = "Read these local files for your complete context.\nIssue JSON: /tmp/ctx/issue.json\nBoard digest: /tmp/ctx/board.txt\nPR diff patch: /tmp/ctx/diff.patch"
    local fix_reflection_prompt = core.build_review_meta_prompt({
      mode = "fix-reflection",
      proposal_id = "github-devloop/issue/owner/repo/42",
      review_proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, "version", "abcdef123456"),
      fix_round = 3,
    }, issue, manifest)
    local judge_prompts = {
      core.build_review_meta_prompt({
        proposal_id = "github-devloop/issue/owner/repo/42",
        review_proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, "version", "abcdef123456"),
      }, issue, manifest),
      fix_reflection_prompt,
    }
    local actor_prompts = {
      core.build_fix_prompt({
        proposal_id = "github-devloop/issue/owner/repo/42",
        review_proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, "version", "abcdef123456"),
        reviewed_head_sha = "abcdef123456",
      }, issue, "Review feedback.", "Approved framing.", manifest),
    }

    for _, prompt in ipairs(judge_prompts) do
      assert_judge_preamble_slots(prompt)
      assert_github_entity_history(prompt)
      t.is_true(prompt:find("/tmp/ctx/issue.json", 1, true) ~= nil)
      t.is_nil(prompt:find("gh issue", 1, true))
      t.is_nil(prompt:find("gh pr", 1, true))
      t.is_nil(prompt:find("gh api", 1, true))
      t.is_nil(prompt:find("{{", 1, true))
    end

    t.is_true(fix_reflection_prompt:find("Line two: the marker named ⟦FKST:REASON⟧ followed by one concise paragraph.", 1, true) ~= nil)
    t.is_true(fix_reflection_prompt:find("You are running in an empty runtime scratch directory", 1, true) ~= nil)
    t.is_true(fix_reflection_prompt:find("Read GitHub context only from the local files named below", 1, true) ~= nil)
    t.is_nil(fix_reflection_prompt:find("Chinese", 1, true))
    t.is_nil(fix_reflection_prompt:find("{{", 1, true))

    for _, prompt in ipairs(actor_prompts) do
      assert_actor_preamble_slots(prompt)
      assert_github_entity_history(prompt)
      t.is_true(prompt:find("/tmp/ctx/issue.json", 1, true) ~= nil)
      t.is_nil(prompt:find("gh issue", 1, true))
      t.is_nil(prompt:find("gh pr", 1, true))
      t.is_nil(prompt:find("gh api", 1, true))
      t.is_nil(prompt:find("empty runtime scratch directory", 1, true))
      t.is_nil(prompt:find("{{", 1, true))
    end
  end,

  test_fix_prompt_carries_agreed_framing_and_merge_context = function()
    local fix = h.fixing({
      framing = "Fix the bounded source_ref migration only; do not raise payload limits.",
      blocking_gap = "rollup red feedback",
    })
    local manifest = "Read these local files for your complete context.\nIssue JSON: /tmp/ctx/issue.json\nBoard digest: /tmp/ctx/board.txt\nPR diff patch: /tmp/ctx/diff.patch"
    local prompt = core.build_fix_prompt(fix, {
      title = "Fix parser",
      body = "Expected behavior",
    }, "Review says the implementation raised the bounds.", fix.framing, manifest)
    t.is_true(prompt:find("Agreed consensus framing", 1, true) ~= nil)
    t.is_true(prompt:find("Fix EXACTLY within this agreed framing", 1, true) ~= nil)
    t.is_true(prompt:find("Fix the bounded source_ref migration only; do not raise payload limits.", 1, true) ~= nil)
    t.is_true(prompt:find("Review says the implementation raised the bounds.", 1, true) ~= nil)
    t.is_nil(prompt:find("Expected behavior", 1, true))
    t.is_true(prompt:find("/tmp/ctx/issue.json", 1, true) ~= nil)
    t.is_nil(prompt:find("gh issue", 1, true))
    t.is_nil(prompt:find("gh pr", 1, true))
    t.is_nil(prompt:find("gh api", 1, true))
    t.is_true(prompt:find("run the local iteration command from the repository root", 1, true) ~= nil)
    t.is_true(prompt:find("local verification is scoped to your change for fast feedback", 1, true) ~= nil)
    t.is_true(prompt:find("CI runs the full `scripts/run.sh test`", 1, true) ~= nil)
    t.is_true(prompt:find("comprehensive gate", 1, true) ~= nil)
    t.is_true(prompt:find("scripts/run.sh test <pkg>", 1, true) ~= nil)
    t.is_true(prompt:find("failing test as the primary signal to fix", 1, true) ~= nil)
    t.is_nil(prompt:find("rerun `scripts/run.sh test` until it exits 0", 1, true))
    t.is_true(prompt:find("Do not finish with failing tests.", 1, true) ~= nil)
    t.is_true(prompt:find("rollup-red feedback", 1, true) ~= nil)
    t.is_true(prompt:find("engine BIN is unreachable", 1, true) ~= nil)
    t.is_true(prompt:find("current target branch has already been merged", 1, true) ~= nil)
    t.is_true(prompt:find("Target branch merge context: sync_clean", 1, true) ~= nil)

    local conflict_prompt = core.build_fix_prompt(fix, {
      title = "Fix parser",
    }, "Review says the implementation raised the bounds.", fix.framing, manifest, {
      target_branch = "dev",
      target_sha = "abc123",
      conflicted = true,
      unmerged_paths = "100644 abc123 1\tpackages/github-devloop/core.lua\n",
    })
    t.is_true(conflict_prompt:find("Target branch merge context: sync_conflict target_branch=dev target_sha=abc123", 1, true) ~= nil)
    t.is_true(conflict_prompt:find("packages/github-devloop/core.lua", 1, true) ~= nil)
  end,

  test_fix_prompt_ignores_full_suite_host_fact_for_local_iteration = function()
    t.mock_command('printf %s "$FKST_DEVLOOP_TEST_COMMAND"', {
      stdout = "cargo build && cargo test",
      stderr = "",
      exit_code = 0,
    })
    local fix = h.fixing({
      framing = "Fix the bounded source_ref migration only.",
    })
    local prompt = core.build_fix_prompt(fix, {
      title = "Fix parser",
    }, "Review says tests are red.", fix.framing)
    t.is_nil(prompt:find("cargo build && cargo test", 1, true))
    t.is_true(prompt:find("run the local iteration command from the repository root", 1, true) ~= nil)
    t.is_true(prompt:find("scripts/run.sh test <pkg>", 1, true) ~= nil)
    t.is_true(prompt:find("CI runs the full `scripts/run.sh test`", 1, true) ~= nil)
  end,

  test_review_meta_action_parser_fails_closed_like_meta_parser = function()
    local clean = meta_answer("fix", "Run another fix pass.", "missing retry guard")
    local parsed = core.parse_review_meta_action(clean)
    t.eq(parsed.action, "fix")
    t.eq(parsed.reason, "Run another fix pass.")
    t.eq(parsed.blocking_gap, "missing retry guard")

    local spec = core.parse_review_meta_action(meta_answer("spec-amendment", "The agreed framing requires unsafe behavior."))
    t.eq(spec.action, "spec-amendment")
    t.eq(spec.reason, "The agreed framing requires unsafe behavior.")
    t.is_nil(spec.blocking_gap)

    t.is_nil(core.parse_review_meta_action(meta_answer("spec-amendment", "The agreed framing requires unsafe behavior.") .. "\ngarbage"))
    t.is_nil(core.parse_review_meta_action(meta_answer("fix", "first") .. "\n" .. meta_answer("block", "second")))
    t.is_nil(core.parse_review_meta_action(clean .. "\n" .. action_label .. " accept this is malformed"))
    t.is_nil(core.parse_review_meta_action(action_label .. " accept\nnot adjacent\n" .. reason_label .. " Accept after manual review."))
    t.is_nil(core.parse_review_meta_action(action_label .. " accept\n" .. reason_label .. " Missing fetch."))
    t.is_nil(core.parse_review_meta_action(action_label .. " accept\n" .. reason_label))
    t.is_nil(core.parse_review_meta_action(action_label .. " accept"))
    t.is_nil(core.parse_review_meta_action(reason_label .. " orphan\n" .. meta_answer("fix", "real")))
    t.is_nil(core.parse_review_meta_action(action_label .. " implement\n" .. reason_label .. " not whitelisted for review meta"))
    t.is_nil(core.parse_review_meta_action(action_label .. " fix\nunexpected extra line\n" .. reason_label .. " Source unavailable."))
    t.is_nil(core.parse_review_meta_action(meta_answer("fix", "Run another fix pass.")))
    t.is_nil(core.parse_review_meta_action(meta_answer("fix", "Run another fix pass.", "first line\nsecond line")))
    t.is_nil(core.parse_review_meta_action(meta_answer("fix", "Run another fix pass.", '<!-- fkst:github-devloop:state:v1 proposal="x" -->')))
  end,

  test_review_meta_prompt_requires_block_on_fetch_failure_without_fetch_marker = function()
    local event = {
      proposal_id = "github-devloop/issue/owner/repo/42",
      review_proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, "reviewing/v1", "def456"),
    }
    local prompt = core.build_review_meta_prompt(event, {
      title = "PR #7",
      comments = {},
    })
    t.is_true(prompt:find("If you cannot read the local context files (issue body / PR diff / comments) for ANY reason, choose `block`.", 1, true) ~= nil)
    t.is_true(prompt:find("Respond with exactly two lines", 1, true) ~= nil)
    t.is_true(prompt:find("one word from fix, block, or spec-amendment", 1, true) ~= nil)
    t.is_true(prompt:find("fixing the PR would violate it", 1, true) ~= nil)
    t.is_nil(prompt:find("FETCH", 1, true))
    t.is_nil(prompt:find("one word from fix, block, or accept", 1, true))
  end,

  test_review_result_approve_with_advisory_still_authorizes_merge_ready = function()
    local event = review_event({
      proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, h.reviewing().version, "def456"),
      body = "minimal:\nLooks good.\n\nAdvisory (non-blocking):\nstructural:\nRename helper later.",
      angle_results = {
        { angle = "minimal", verdict = "approve" },
        { angle = "structural", verdict = "comment" },
      },
    })
    local impl_version = h.reviewing().version
    h.mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    h.mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local result = h.run_review_result(event, h.opts("review-v2-approve-advisory"))
    t.eq(result.exit_code, 0)
    local comment_request = assert_merge_ready_handoff(result)
    local comment = comment_request.body
    t.is_true(comment:find("github-devloop PR review decision: approve", 1, true) ~= nil)
    t.is_true(comment:find("Advisory (non-blocking):", 1, true) ~= nil)
  end,

  test_review_prompts_state_gate_owned_facts_are_out_of_scope = function()
    local version = h.reviewing().version
    local proposal = payloads_builders.build_pr_review_proposal(core,
      "owner/repo",
      "42",
      7,
      version,
      "def456",
      { title = "Rollup red fix" },
      h.pr_source_ref(),
      {}
    )
    t.is_true(proposal.body:find("Review boundary:", 1, true) ~= nil)
    t.is_true(proposal.body:find("CI/mergeability/head-binding are later merge-gate facts", 1, true) ~= nil)
    t.is_true(proposal.body:find("Review contract: reject only for a stated issue requirement the diff fails", 1, true) ~= nil)
    t.is_true(#proposal.body < 512)

    local reject_comment = requests_review.build_review_result_comment_request(core,
      "owner/repo",
      "42",
      "github-devloop/issue/owner/repo/42",
      version,
      {
        proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, version, "def456"),
        decision = "reject",
        body = "Reject body.",
        blocking_gap = "CI green evidence is missing for the current head.",
        dedup_key = "consensus:" .. devloop_base.pr_review_proposal_id("owner/repo", 7, version, "def456") .. "/review",
        source_ref = h.pr_source_ref(),
      },
      h.pr_source_ref()
    ).body
    local fix_comment = requests_review.build_fix_reviewing_comment_request(core,
      "owner/repo",
      "42",
      {
        proposal_id = "github-devloop/issue/owner/repo/42",
        pr_number = 7,
        review_proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, version, "def456"),
        review_dedup_key = "consensus:" .. devloop_base.pr_review_proposal_id("owner/repo", 7, version, "def456") .. "/review",
        source_ref = h.pr_source_ref(),
        fix_summary = "Reproduced the failing check locally and changed the diff.",
      },
      "def456",
      "feedface",
      core.next_fix_version(version)
    ).body
    local rereview = payloads_builders.build_pr_review_proposal(core,
      "owner/repo",
      "42",
      7,
      core.next_fix_version(version),
      "feedface",
      { title = "Rollup red fix" },
      h.pr_source_ref(),
      {
        { body = reject_comment, author_login = "fkst-test-bot" },
        { body = fix_comment, author_login = "fkst-test-bot" },
      }
    )
    t.is_true(rereview.body:find("named failing check, not to restoration of gate state", 1, true) ~= nil)

    local fix_prompt = core.build_fix_prompt(h.fixing({
      blocking_gap = "CI green evidence is missing for the current head.",
    }), { title = "Rollup red fix" }, "Reject prose.", "Approved framing.")
    t.is_true(fix_prompt:find("OUT OF REVIEW SCOPE", 1, true) ~= nil)

    local meta_prompt = core.build_review_meta_prompt(h.review_meta_event(), {
      title = "Rollup red fix",
      comments = {},
    })
    t.is_true(meta_prompt:find("gate-owned fact", 1, true) ~= nil)
    t.is_true(meta_prompt:find("not as a reason for another fix pass", 1, true) ~= nil)
    t.is_true(meta_prompt:find("cites no stated issue requirement", 1, true) ~= nil)
    t.is_true(meta_prompt:find("spec-amendment material, not as fix material", 1, true) ~= nil)
  end,

  test_out_of_contract_reject_is_advisory_and_does_not_enter_fixing = function()
    local event = review_event({
      decision = "reject",
      body = "Reject: add an immutability proof before merge.",
      blocking_gap = "New requirement outside the stated issue acceptance bounds: prove API immutability.",
      angle_results = {
        { angle = "minimal", verdict = "approve" },
        { angle = "delete", verdict = "reject" },
      },
    })
    local impl_version = h.reviewing().version
    h.mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    h.mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local result = h.run_review_result(event, h.opts("review-v2-contract-gap-advisory"))
    t.eq(result.exit_code, 0)
    t.is_nil(h.find_raise(result.raises, "devloop_fixing"))
    local comment_request = assert_merge_ready_handoff(result)
    local comment = comment_request.body
    t.is_true(comment:find("github-devloop PR review decision: approve", 1, true) ~= nil)
    t.is_true(comment:find("Advisory (out-of-contract): rejected only for demand beyond the stated issue bounds", 1, true) ~= nil)
    t.is_true(comment:find("merge-ready", 1, true) ~= nil)
  end,

  test_pr_body_evidence_gap_is_advisory_and_does_not_enter_fixing = function()
    local event = review_event({
      decision = "reject",
      body = "Reject: the diff is acceptable, but the pull request description lacks duplicate-evidence analysis.",
      blocking_gap = "Missing PR body duplicate-evidence analysis",
      angle_results = {
        { angle = "minimal", verdict = "approve" },
        { angle = "structural", verdict = "reject" },
        { angle = "delete", verdict = "approve" },
      },
    })
    local impl_version = h.reviewing().version
    h.mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    h.mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local result = h.run_review_result(event, h.opts("review-v2-pr-body-gap-advisory"))
    t.eq(result.exit_code, 0)
    t.is_nil(h.find_raise(result.raises, "devloop_fixing"))
    local comment_request = assert_merge_ready_handoff(result)
    local comment = comment_request.body
    t.is_true(comment:find("github-devloop PR review decision: approve", 1, true) ~= nil)
    t.is_true(comment:find("Advisory (out-of-contract): rejected only for demand beyond the stated issue bounds", 1, true) ~= nil)
    t.is_true(comment:find("Missing PR body duplicate-evidence analysis", 1, true) ~= nil)
  end,

  test_gate_owned_reject_is_advisory_and_does_not_enter_fixing = function()
    local event = review_event({
      decision = "reject",
      body = "Reject: current head has no green merge-gate evidence.",
      blocking_gap = "缺少当前 head 的权威绿色合并门证据",
    })
    local impl_version = h.reviewing().version
    h.mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    h.mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local result = h.run_review_result(event, h.opts("review-v2-gate-gap-advisory"))
    t.eq(result.exit_code, 0)
    t.is_nil(h.find_raise(result.raises, "devloop_fixing"))
    local comment_request = assert_merge_ready_handoff(result)
    local comment = comment_request.body
    t.is_true(comment:find("github-devloop PR review decision: approve", 1, true) ~= nil)
    t.is_true(comment:find("Advisory (out-of-contract): rejected only for gate-owned fact", 1, true) ~= nil)
  end,

  test_gate_fact_plus_implementation_gap_still_enters_fixing = function()
    local event = review_event({
      decision = "reject",
      body = "Reject: test guard is missing.",
      blocking_gap = "Missing test guard; CI green evidence is also absent.",
    })
    local impl_version = h.reviewing().version
    h.mock_pr_origin({
      m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    h.mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", impl_version),
    })

    local result = h.run_review_result(event, h.opts("review-v2-gate-plus-code-gap"))
    t.eq(result.exit_code, 0)
    t.is_true(h.find_causal_raise(result, "devloop_fixing") ~= nil)
    t.is_nil(h.find_raise(result.raises, "devloop_merge_ready"))
  end,

  test_reject_without_blocking_gap_fails_closed_before_fixing = function()
    local event = review_event({
      decision = "reject",
    })
    event.blocking_gap = nil
    local impl_version = h.reviewing().version
    h.mock_pr_origin({
      m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })

    local result = h.run_review_result(event, h.opts("review-v2-reject-missing-gap"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_fix_prompt_uses_named_gap_and_ledger_feeds_post_fix_review = function()
    local fix = h.fixing({ blocking_gap = "missing rollback guard" })
    local prompt = core.build_fix_prompt(fix, { title = "Issue title" }, "Reject prose with advisory.", "Approved framing.")
    t.is_true(prompt:find("Apply the SMALLEST change that closes the named blocking gap: missing rollback guard", 1, true) ~= nil)
    t.is_true(prompt:find("Do not address advisory comments.", 1, true) ~= nil)
    t.is_true(prompt:find("State in your summary which gap you closed.", 1, true) ~= nil)

    local reject_comment = requests_review.build_review_result_comment_request(core,
      "owner/repo",
      "42",
      fix.proposal_id,
      fix.version,
      {
        proposal_id = fix.review_proposal_id,
        decision = "reject",
        body = "Reject body.",
        blocking_gap = "missing rollback guard",
        dedup_key = fix.review_dedup_key,
        source_ref = fix.source_ref,
      },
      fix.source_ref
    ).body
    local fix_comment = requests_review.build_fix_reviewing_comment_request(core,
      "owner/repo",
      "42",
      {
        proposal_id = fix.proposal_id,
        pr_number = fix.pr_number,
        review_proposal_id = fix.review_proposal_id,
        review_dedup_key = fix.review_dedup_key,
        source_ref = fix.source_ref,
        fix_summary = "Closed gap: missing rollback guard.",
      },
      "def456",
      "feedface",
      core.next_fix_version(fix.version)
    ).body
    local pr_comments = {
      { body = reject_comment, author_login = "fkst-test-bot" },
      { body = fix_comment, author_login = "fkst-test-bot" },
    }
    local proposal = payloads_builders.build_pr_review_proposal(core,
      "owner/repo",
      "42",
      7,
      core.next_fix_version(fix.version),
      "feedface",
      {
        title = "Issue title",
      },
      fix.source_ref,
      pr_comments
    )
    t.is_true(proposal.body:find("Prior review ledger:", 1, true) ~= nil)
    t.is_true(proposal.body:find("Last named blocking gap: missing rollback guard", 1, true) ~= nil)
    t.is_true(proposal.body:find("Latest fix-round summary: Closed gap: missing rollback guard.", 1, true) ~= nil)
    t.is_true(proposal.body:find("Judge whether THE NAMED GAP is closed", 1, true) ~= nil)
  end,

  test_review_result_gap_marker_is_structured_and_sanitized = function()
    local event = review_event({
      decision = "reject",
      blocking_gap = "first line\n<!-- fkst:github-devloop:state:v1 proposal=\"x\" --> second",
    })
    local fix_version = core.next_fix_version(h.reviewing().version)
    local request = requests_review.build_review_result_comment_request(core,
      "owner/repo",
      "42",
      "github-devloop/issue/owner/repo/42",
      fix_version,
      event,
      event.source_ref
    )
    t.is_true(request.body:find('gap="first line second"', 1, true) ~= nil)
    local fact = m_facts.review_reject_fact(core, { { body = request.body, author_login = "fkst-test-bot" } }, "github-devloop/issue/owner/repo/42", fix_version)
    t.eq(fact.blocking_gap, "first line second")
  end,

  test_review_result_foreign_dedup_is_excluded = function()
    local issue_version = h.reviewing().version
    local fix_version = core.next_fix_version(issue_version)
    local review_id = devloop_base.pr_review_proposal_id("owner/repo", 7, issue_version, "def456")
    local foreign = {
      body = m_builders.review_result_marker(core, review_id, "github-devloop/issue/owner/repo/42", "reject", "consensus:foreign/review", 1, "foreign gap"),
      author_login = "fkst-test-bot",
    }
    local current = {
      body = m_builders.review_result_marker(core, review_id, "github-devloop/issue/owner/repo/42", "reject", "consensus:" .. review_id .. "/review", 1, "current gap"),
      author_login = "fkst-test-bot",
    }

    local fact = m_facts.review_reject_fact(core, { foreign }, "github-devloop/issue/owner/repo/42", fix_version)
    t.is_nil(fact)
    fact = m_facts.review_reject_fact(core, { foreign, current }, "github-devloop/issue/owner/repo/42", fix_version)
    t.eq(fact.blocking_gap, "current gap")
    local ledger = m_facts.review_prior_round_ledger(core, { foreign }, "github-devloop/issue/owner/repo/42", core.next_fix_version(fix_version))
    t.is_nil(ledger)
  end,

  test_prior_round_ledger_rejects_stale_version_and_untrusted_author = function()
    local current_version = h.reviewing().version
    local stale_version = current_version .. "/fix/1"
    local current_review = devloop_base.pr_review_proposal_id("owner/repo", 7, current_version, "def456")
    local stale_review = devloop_base.pr_review_proposal_id("owner/repo", 7, stale_version, "def456")
    local trusted_stale = {
      body = m_builders.review_result_marker(core, stale_review, "github-devloop/issue/owner/repo/42", "reject", "consensus:" .. stale_review .. "/review", 1, "stale gap"),
      author_login = "fkst-test-bot",
    }
    local untrusted_current = {
      body = m_builders.review_result_marker(core, current_review, "github-devloop/issue/owner/repo/42", "reject", "consensus:" .. current_review .. "/review", 0, "untrusted gap"),
      author_login = "mallory",
    }
    local fix_version = core.next_fix_version(current_version)
    t.is_nil(m_facts.review_prior_round_ledger(core, { trusted_stale, untrusted_current }, "github-devloop/issue/owner/repo/42", fix_version))
  end,

  test_prior_round_ledger_uses_highest_round_when_comments_are_out_of_order = function()
    local base_version = h.reviewing().version
    local round1_fix = core.next_fix_version(base_version)
    local round2_fix = core.next_fix_version(round1_fix)
    local round3_fix = core.next_fix_version(round2_fix)
    local round1_review = devloop_base.pr_review_proposal_id("owner/repo", 7, base_version, "def456")
    local round2_review = devloop_base.pr_review_proposal_id("owner/repo", 7, round2_fix, "feedface")
    local round1 = {
      body = m_builders.review_result_marker(core, round1_review, "github-devloop/issue/owner/repo/42", "reject", "consensus:" .. round1_review .. "/review", 1, "round one gap")
        .. "\nFix-round summary: Closed round one."
        .. "\n" .. core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", round2_fix),
      author_login = "fkst-test-bot",
    }
    local round2 = {
      body = m_builders.review_result_marker(core, round2_review, "github-devloop/issue/owner/repo/42", "reject", "consensus:" .. round2_review .. "/review", 3, "round three gap")
        .. "\nFix-round summary: Closed round three."
        .. "\n" .. core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", round3_fix),
      author_login = "fkst-test-bot",
    }

    local ledger = m_facts.review_prior_round_ledger(core, { round2, round1 }, "github-devloop/issue/owner/repo/42", core.next_fix_version(round3_fix))
    t.is_true(ledger:find("Last named blocking gap: round three gap", 1, true) ~= nil)
    t.is_true(ledger:find("Latest fix-round summary: Closed round three.", 1, true) ~= nil)
    t.is_nil(ledger:find("round one", 1, true))
  end,

  test_prior_round_ledger_truncates_utf8_safely = function()
    local base_version = h.reviewing().version
    local fix_version = core.next_fix_version(base_version)
    local review = devloop_base.pr_review_proposal_id("owner/repo", 7, base_version, "def456")
    local cjk = fixtures.cjk_char()
    local reject = {
      body = m_builders.review_result_marker(core, 
        review,
        "github-devloop/issue/owner/repo/42",
        "reject",
        "consensus:" .. review .. "/review",
        1,
        string.rep("a", core._max_blocking_gap_len - 1) .. cjk
      ),
      author_login = "fkst-test-bot",
    }
    local fix = {
      body = "Fix-round summary: " .. string.rep("b", core._max_review_ledger_len - 1) .. cjk
        .. "\n" .. core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", fix_version),
      author_login = "fkst-test-bot",
    }

    local ledger = m_facts.review_prior_round_ledger(core, { reject, fix }, "github-devloop/issue/owner/repo/42", core.next_fix_version(fix_version))
    assert_valid_utf8(ledger)
    t.is_true(#ledger <= core._max_review_ledger_len)
  end,

  test_prior_round_ledger_reads_pr_stream_not_issue_stream = function()
    local fix = h.fixing({ blocking_gap = "missing rollback guard" })
    local reject_comment = requests_review.build_review_result_comment_request(core,
      "owner/repo",
      "42",
      fix.proposal_id,
      fix.version,
      {
        proposal_id = fix.review_proposal_id,
        decision = "reject",
        body = "Reject body.",
        blocking_gap = "missing rollback guard",
        dedup_key = fix.review_dedup_key,
        source_ref = fix.source_ref,
      },
      fix.source_ref
    ).body
    local proposal = payloads_builders.build_pr_review_proposal(core,
      "owner/repo",
      "42",
      7,
      core.next_fix_version(fix.version),
      "feedface",
      {
        title = "Issue title",
        comments = {
          { body = reject_comment, author_login = "fkst-test-bot" },
        },
      },
      fix.source_ref,
      {}
    )
    t.is_nil(proposal.body:find("Prior review ledger:", 1, true))
  end,
}
