local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local source_ref = h.source_ref
local reached = h.reached
local unresolved = h.unresolved

local ai_sentinel = string.char(226, 159, 166) .. "AI:FKST" .. string.char(226, 159, 167)
local cjk_probe = string.char(228, 184, 173)

local issue_proposal_id = "github-devloop/issue/owner/repo/42"
local issue_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local review_proposal_id = core.pr_review_proposal_id("owner/repo", 7, issue_version, "def456")
local review_dedup_key = "consensus:" .. review_proposal_id .. "/review"
local pr_source_ref = {
  kind = "external",
  ref = "owner/repo#pr/7",
}

local function review_reached(extra)
  local value = {
    schema = "consensus.consensus_reached.v1",
    proposal_id = review_proposal_id,
    decision = "approve",
    body = "Review consensus approves the diff.",
    dedup_key = review_dedup_key,
    source_ref = pr_source_ref,
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function review_unresolved(extra)
  local value = {
    schema = "consensus.consensus_converge.v1",
    proposal_id = review_proposal_id,
    dedup_key = review_dedup_key,
    source_ref = pr_source_ref,
    pr_number = 7,
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function ready_payload()
  return core.build_devloop_ready_payload(reached())
end

local function merge_ready_payload()
  return {
    schema = "github-devloop.merge-ready.v1",
    proposal_id = issue_proposal_id,
    version = issue_version,
    dedup_key = "merge-ready:" .. issue_version,
    pr_number = 7,
    review_proposal_id = review_proposal_id,
    review_dedup_key = review_dedup_key,
    reviewed_head_sha = "def456",
    source_ref = pr_source_ref,
  }
end

local function review_meta_payload()
  return {
    schema = "github-devloop.review-meta.v1",
    proposal_id = issue_proposal_id,
    version = issue_version,
    dedup_key = "review-meta:" .. issue_version,
    review_proposal_id = review_proposal_id,
    pr_number = 7,
    source_ref = pr_source_ref,
  }
end

local function collect_markers(body)
  local markers = {}
  for marker in tostring(body or ""):gmatch("<!%-%- fkst:github%-devloop:.-%-%->") do
    table.insert(markers, marker)
  end
  return table.concat(markers, "\n")
end

local function strip_markers(body)
  return tostring(body or ""):gsub("<!%-%- fkst:github%-devloop:.-%-%->", "")
end

local function extract_machine(body)
  local machine = collect_markers(body)
  if tostring(body or ""):find(ai_sentinel, 1, true) ~= nil then
    machine = machine .. "\n" .. ai_sentinel
  end
  return machine
end

local function comment_cases()
  local ready = ready_payload()
  local reached_with_angles = reached({
    angle_results = {
      { angle = "minimal", verdict = "approve" },
      { angle = "structural", verdict = "abstain" },
      { angle = "delete", verdict = "approve" },
    },
  })
  local converge_marker = core.converge_round_marker(
    issue_proposal_id,
    reached_with_angles.dedup_key,
    core.source_ref_digest(source_ref()),
    2,
    reached_with_angles.dedup_key .. "/loop/2",
    "Narrow question?",
    { { angle = "minimal", verdict = "abstain", digest = "digest" } }
  )
  local review_converge_marker = core.review_converge_round_marker(
    review_proposal_id,
    issue_proposal_id,
    issue_version,
    "def456",
    core.source_ref_digest(pr_source_ref),
    2,
    review_dedup_key .. "/loop/2",
    "Review question?",
    { { angle = "minimal", verdict = "abstain", digest = "digest" } }
  )
  local reconcile = core.build_devloop_reconcile_payload(unresolved(), 3, reached_with_angles.dedup_key)
  local review_reconcile = core.build_devloop_review_reconcile_payload(review_unresolved(), 3, issue_proposal_id, issue_version, "def456")
  local fix_reconcile = core.build_devloop_fix_reconcile_payload({
    proposal_id = issue_proposal_id,
    review_proposal_id = review_proposal_id,
    review_dedup_key = review_dedup_key,
    reviewed_head_sha = "def456",
    pr_number = 7,
    source_ref = pr_source_ref,
  }, issue_version .. "/fix/4")
  local origin = {
    proposal_id = issue_proposal_id,
    impl_version = ready.dedup_key,
  }
  local fix = {
    proposal_id = issue_proposal_id,
    version = issue_version,
    review_proposal_id = review_proposal_id,
    review_dedup_key = review_dedup_key,
    reviewed_head_sha = "def456",
    pr_number = 7,
    fix_summary = "Closed the review gap.",
    dedup_key = "fix:" .. issue_version,
    source_ref = pr_source_ref,
  }
  local gate = { kind = "waiting", reason = "waiting-on-dependency" }
  local dependency_marker = core.dependency_wait_marker(issue_proposal_id, issue_version, { 7 }, gate.kind, gate.reason)
  local dependency_void_gate = {
    kind = "satisfied",
    reason = "dependency-void",
    notes = {
      { kind = "dependency-void", blocker_number = 7, reason = "not_planned" },
    },
  }
  local merge_ready = merge_ready_payload()
  local decompose = {
    proposal_id = issue_proposal_id,
    version = issue_version,
    pr_number = 7,
  }

  return {
    { id = "thinking", request = core.build_observe_comment_request({ repo = "owner/repo", number = 42, source_ref = source_ref() }, { proposal_id = issue_proposal_id, dedup_key = "v1" }) },
    { id = "result", request = core.build_result_comment_request("owner/repo", "42", reached_with_angles) },
    { id = "converge", request = core.build_converge_round_comment_request("owner/repo", "42", unresolved({
      narrowed_question = "Narrow question?",
      angle_digests = { { angle = "minimal", verdict = "abstain", digest = "digest" } },
    }), 2, converge_marker) },
    { id = "review-converge-pr", request = core.build_review_converge_round_comment_request("owner/repo", "42", review_unresolved({
      narrowed_question = "Review question?",
      angle_digests = { { angle = "minimal", verdict = "abstain", digest = "digest" } },
    }), issue_proposal_id, 2, review_converge_marker) },
    { id = "review-converge-issue", request = core.build_issue_review_converge_round_comment_request("owner/repo", "42", review_unresolved({
      narrowed_question = "Review question?",
      angle_digests = { { angle = "minimal", verdict = "abstain", digest = "digest" } },
    }), issue_proposal_id, 2, review_converge_marker) },
    { id = "reconcile", request = core.build_reconcile_comment_request("owner/repo", "42", reconcile, "drop", "no-actionable-framing") },
    { id = "fix-reconcile", request = core.build_fix_reconcile_comment_request("owner/repo", "42", fix_reconcile, "drop", "fix-loop-max-rounds") },
    { id = "review-reconcile", request = core.build_review_reconcile_comment_request("owner/repo", "42", review_reconcile, "drop", "review-loop-stalled") },
    { id = "intake", request = core.build_intake_decision_comment_request("owner/repo", "42", { proposal_id = issue_proposal_id, dedup_key = "intake:v1", source_ref = source_ref() }, "enable", "clear code change", "standard") },
    { id = "implementing", request = core.build_implementing_comment_request("owner/repo", "42", ready, "/tmp/worktree", "devloop-owner-repo-42", "abc123", "dev", "abc123") },
    { id = "impl-failure", request = core.build_impl_failure_comment_request("owner/repo", "42", ready, "no-changes", "") },
    { id = "pr-open-template", request = { body = core.build_pr_open_request("owner/repo", "42", issue_proposal_id, { state = "implementing", version = ready.dedup_key }, "Implement", "devloop-owner-repo-42", "abc123", "dev").issue_comment_body_template } },
    { id = "pr-open", request = core.build_pr_open_comment_request("owner/repo", "42", issue_proposal_id, { version = ready.dedup_key }, 7, "devloop-owner-repo-42", "dev", source_ref()) },
    { id = "reviewing", request = core.build_reviewing_comment_request("owner/repo", "42", origin, 7, pr_source_ref) },
    { id = "review-result-approve", request = core.build_review_result_comment_request("owner/repo", "42", issue_proposal_id, issue_version, review_reached(), pr_source_ref) },
    { id = "review-result-reject", request = core.build_review_result_comment_request("owner/repo", "42", issue_proposal_id, issue_version .. "/fix/1", review_reached({ decision = "reject", blocking_gap = "missing guard" }), pr_source_ref) },
    { id = "merge-gate", request = core.build_merge_gate_fix_comment_request("owner/repo", "42", merge_ready, issue_version .. "/fix/1", "rollup-red", "abc123", pr_source_ref) },
    { id = "fix-reviewing", request = core.build_fix_reviewing_comment_request("owner/repo", "42", fix, "def456", "abc123", issue_version .. "/fix/1") },
    { id = "merge-head-reviewing", request = core.build_merge_head_reviewing_comment_request("owner/repo", "42", merge_ready, "def456", "abc123", issue_version .. "/head-advanced", pr_source_ref) },
    { id = "fix-review-meta", request = core.build_fix_review_meta_comment_request("owner/repo", "42", fix, "no-fix", "") },
    { id = "review-meta", request = core.build_review_meta_comment_request("owner/repo", "42", review_meta_payload(), "fix", "Run another fix pass.", issue_version .. "/fix/1", "missing guard") },
    { id = "dependency-hold", request = core.build_dependency_hold_comment_request("owner/repo", "42", issue_proposal_id, issue_version, gate, dependency_marker, source_ref()) },
    { id = "dependency-release", request = core.build_dependency_release_comment_request("owner/repo", "42", issue_proposal_id, issue_version, dependency_void_gate, source_ref()) },
    { id = "merging", request = { body = core.build_merging_comment_body(merge_ready) } },
    { id = "merged", request = { body = core.build_merged_comment_body(merge_ready) } },
    { id = "decomposed", request = { body = core.decomposed_comment_body(decompose, 2) } },
  }
end

local audited_english_skeletons = {
  "github-devloop thinking: consensus started",
  "github-devloop decision: ",
  "Three-angle verdicts: ",
  "github-devloop convergence round ",
  "github-devloop PR review convergence round ",
  "Narrowed question: ",
  "Angle stances:",
  "github-devloop reconcile action: ",
  "github-devloop fix reconcile action: ",
  "github-devloop review reconcile action: ",
  "github-devloop intake decision: ",
  "Reason:",
  "(no reason provided)",
  "github-devloop implementation started",
  "Worktree: ",
  "Branch: ",
  "Head: ",
  "Base branch: ",
  "Base head: ",
  "github-devloop implementation failed: ",
  "(no implementation output)",
  "github-devloop PR opened: #",
  "github-devloop PR is ready for review",
  "github-devloop PR review decision: ",
  "Blocking gap: ",
  "github-devloop merge gate failed: ",
  "Reproduce locally with `",
  "` from the repository root.",
  "Fix-round summary: ",
  "github-devloop fix pushed for re-review",
  "Previous reviewed head: ",
  "New head: ",
  "Current head: ",
  "github-devloop PR head advanced after merge approval; re-entering review",
  "github-devloop fix escalated to review-meta: ",
  "github-devloop review-meta action: ",
  "github-devloop dependency hold: ",
  "github-devloop dependency release: ",
  "Acknowledged as a tracking umbrella. Individual waves should enter the pipeline as separate issues; this issue stays open for tracking.",
  "github-devloop is merging PR #",
  "github-devloop merged PR #",
  "github-devloop decomposed blocked PR into ",
  " follow-up issue(s)",
}

local function render_cases(lang)
  core.configure_output_lang(lang)
  local rendered = comment_cases()
  core.configure_output_lang(nil)
  return rendered
end

local function body_of(case)
  return case.request.body
end

local function argv_option(argv, name)
  for index, value in ipairs(argv or {}) do
    if value == name then
      return argv[index + 1]
    end
  end
  return nil
end

return {
  test_release_notes_pr_create_debug_stamp_is_default_off = function()
    t.mock_command('printf %s "$FKST_DEBUG_STAMP"', { stdout = "" })
    local seen
    local old_exec_argv = exec_argv
    exec_argv = function(spec)
      if spec.argv[1] == "git" then
        return { stdout = "0123456789ABCDEF\n", stderr = "", exit_code = 0 }
      end
      seen = spec
      return { stdout = "https://github.example/owner/repo/pull/1\n", stderr = "", exit_code = 0 }
    end

    local ok, err = pcall(function()
      core.gh_pr_create_body("owner/repo", "integration-x", "dev", "rollup", "Release notes")
    end)
    exec_argv = old_exec_argv
    if not ok then error(err) end

    t.eq(seen.argv[1], "gh")
    t.is_nil(argv_option(seen.argv, "--body"):find("fkst:debug-stamp:v1", 1, true))
  end,

  test_release_notes_pr_create_debug_stamp_is_enabled_and_redacted = function()
    t.mock_command('printf %s "$FKST_DEBUG_STAMP"', { stdout = "1" })
    t.mock_command("git rev-parse --verify HEAD", {
      stdout = "0123456789ABCDEF\n",
      stderr = "",
      exit_code = 0,
    })
    local seen
    local old_exec_argv = exec_argv
    exec_argv = function(spec)
      if spec.argv[1] == "git" then
        return { stdout = "0123456789ABCDEF\n", stderr = "", exit_code = 0 }
      end
      seen = spec
      return { stdout = "https://github.example/owner/repo/pull/1\n", stderr = "", exit_code = 0 }
    end

    local ok, err = pcall(function()
      core.gh_pr_create_body("owner/repo", "integration-x", "dev", "rollup", "Release notes")
    end)
    exec_argv = old_exec_argv
    if not ok then error(err) end

    local rendered = argv_option(seen.argv, "--body")
    t.is_true(rendered:find("fkst:debug-stamp:v1", 1, true) ~= nil)
    t.is_true(rendered:find('emitter="github-devloop.rollup.pr-create"', 1, true) ~= nil)
    t.is_true(rendered:find('target="pr:owner/repo#new"', 1, true) ~= nil)
    t.is_true(rendered:find('code_version="0123456789abcdef"', 1, true) ~= nil)
    t.is_true(rendered:find('dedup_hash="', 1, true) ~= nil)
    t.is_nil(rendered:find("integration-x->dev", 1, true))
  end,

  test_comment_template_audit_has_complete_language_table = function()
    local en = core.comment_strings("en")
    local zh = core.comment_strings("zh")
    local human = 0
    for _, row in ipairs(core.comment_template_audit()) do
      if row.classification == "human" then
        human = human + 1
        t.is_true(en[row.id] ~= nil)
        t.is_true(zh[row.id] ~= nil)
        t.eq(zh[row.id] ~= en[row.id], true)
      else
        t.is_true(row.classification == "machine" or row.classification == "repo-policy")
      end
    end
    t.is_true(human >= #audited_english_skeletons - 2)
  end,

  test_zh_comments_localize_human_skeletons_and_keep_machine_tokens = function()
    local en_cases = render_cases("en")
    local zh_cases = render_cases("zh")
    t.eq(#en_cases, #zh_cases)
    local localized_count = 0
    for index, en_case in ipairs(en_cases) do
      local zh_case = zh_cases[index]
      t.eq(zh_case.id, en_case.id)
      t.eq(zh_case.request.dedup_key, en_case.request.dedup_key)
      t.eq(extract_machine(body_of(zh_case)), extract_machine(body_of(en_case)))
      if strip_markers(body_of(zh_case)) ~= strip_markers(body_of(en_case)) then
        localized_count = localized_count + 1
      end
      for _, english in ipairs(audited_english_skeletons) do
        t.eq(strip_markers(body_of(zh_case)):find(english, 1, true), nil)
      end
    end
    t.eq(localized_count, #zh_cases)
  end,

  test_parsers_anchor_on_machine_tokens_not_prose = function()
    local issue_comments = {
      {
        body = "lorem ipsum " .. cjk_probe .. "\n"
          .. core.state_marker(issue_proposal_id, "ready", issue_version)
          .. "\n" .. core.result_marker(issue_proposal_id, "approve", "consensus:v1")
          .. "\n" .. core.dependency_wait_marker(issue_proposal_id, issue_version, { 7 }),
        author_login = core.trusted_bot_login(),
      },
    }
    local review_comments = {
      {
        body = "noise only " .. cjk_probe .. "\n"
          .. core.state_marker(issue_proposal_id, "fixing", issue_version .. "/fix/1")
          .. "\n"
          .. core.review_result_marker(review_proposal_id, issue_proposal_id, "reject", review_dedup_key, 1, "missing guard")
          .. "\n" .. core.merge_ready_marker(issue_proposal_id, 7, issue_version, review_proposal_id, review_dedup_key, "def456")
          .. "\n" .. core.review_meta_marker(issue_proposal_id, review_dedup_key, "fix", issue_version .. "/fix/1", "missing guard")
          .. "\n" .. core.merge_gate_marker(issue_proposal_id, 7, issue_version .. "/fix/1", review_proposal_id, review_dedup_key, "def456", "abc123", "rollup-red"),
        author_login = core.trusted_bot_login(),
      },
    }
    local implementation_comments = {
      {
        body = "more noise " .. cjk_probe .. "\n"
          .. core.implementing_marker(issue_proposal_id, "impl:v1", "devloop-owner-repo-42", "abc123", "dev", "abc123")
          .. "\n" .. core.pr_link_marker(issue_proposal_id, 7, "devloop-owner-repo-42", "impl:v1", "dev")
          .. "\n" .. core.impl_failure_marker(issue_proposal_id, "impl:v1", "codex-failed"),
        author_login = core.trusted_bot_login(),
      },
    }

    t.eq(core.current_state(issue_comments, issue_proposal_id).state, "ready")
    t.eq(core.has_result_marker(issue_comments, issue_proposal_id, "approve", "consensus:v1"), true)
    t.eq(core.dependency_hold_fact(issue_comments, issue_proposal_id).marker_kind, "dependency-wait")
    t.eq(core.dependency_waiver_fact({
      {
        body = "noise " .. cjk_probe .. "\n"
          .. core.dependency_waiver_marker(issue_proposal_id, issue_version, 7, "operator-waiver"),
        author_login = core.trusted_bot_login(),
      },
    }, issue_proposal_id, issue_version, 7).reason, "operator-waiver")
    t.eq(core.review_reject_fact(review_comments, issue_proposal_id, issue_version .. "/fix/1").blocking_gap, "missing guard")
    t.eq(core.review_meta_fix_fact(review_comments, issue_proposal_id, issue_version .. "/fix/1").blocking_gap, "missing guard")
    t.eq(core.merge_gate_fix_fact(review_comments, issue_proposal_id, issue_version .. "/fix/1").reviewed_head_sha, "def456")
    t.eq(core.merge_gate_fix_fact(review_comments, issue_proposal_id, issue_version .. "/fix/1").gate_baseline_sha, "abc123")
    t.eq(core.merge_ready_fact(review_comments, issue_proposal_id, issue_version, 7).head_sha, "def456")
    t.eq(core.implementing_fact(implementation_comments, issue_proposal_id, "impl:v1").branch, "devloop-owner-repo-42")
    t.eq(core.pr_link_fact(implementation_comments, issue_proposal_id).pr_number, 7)
    t.eq(core.has_impl_failure_marker(implementation_comments, issue_proposal_id, "impl:v1"), true)
  end,
}
