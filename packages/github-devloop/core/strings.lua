local S = {}

function S.install(M)
local function h(hex)
  return (tostring(hex or ""):gsub("..", function(byte)
    return string.char(tonumber(byte, 16))
  end))
end

local strings = {
  en = {
    convergence_suffix = " - no three-angle consensus; narrowing",
    narrowed_question_label = "Narrowed question: ",
    angle_stances_label = "Angle stances:",
    verdict_summary_label = "Three-angle verdicts: ",
    comment_evidence_empty = "(review rounds are recorded on the parent PR comments)",
    thinking_started = "github-devloop thinking: consensus started",
    decision_prefix = "github-devloop decision: ",
    convergence_round_prefix = "github-devloop convergence round ",
    pr_review_convergence_round_prefix = "github-devloop PR review convergence round ",
    reconcile_action_prefix = "github-devloop reconcile action: ",
    fix_reconcile_action_prefix = "github-devloop fix reconcile action: ",
    review_reconcile_action_prefix = "github-devloop review reconcile action: ",
    reason_block_label = "Reason:",
    reason_inline_label = "Reason: ",
    no_reason_provided = "(no reason provided)",
    implementation_started = "github-devloop implementation started",
    worktree_label = "Worktree: ",
    branch_label = "Branch: ",
    head_label = "Head: ",
    base_branch_label = "Base branch: ",
    base_head_label = "Base head: ",
    implementation_failed_prefix = "github-devloop implementation failed: ",
    no_implementation_output = "(no implementation output)",
    pr_opened_prefix = "github-devloop PR opened: #",
    pr_ready_for_review = "github-devloop PR is ready for review",
    pr_review_decision_prefix = "github-devloop PR review decision: ",
    blocking_gap_label = "Blocking gap: ",
    merge_gate_failed_prefix = "github-devloop merge gate failed: ",
    reproduce_locally_prefix = "Reproduce locally with `",
    reproduce_locally_suffix = "` from the repository root.",
    fix_round_summary_label = "Fix-round summary: ",
    fix_pushed_for_rereview = "github-devloop fix pushed for re-review",
    previous_reviewed_head_label = "Previous reviewed head: ",
    new_head_label = "New head: ",
    current_head_label = "Current head: ",
    pr_head_advanced = "github-devloop PR head advanced after merge approval; re-entering review",
    fix_escalated_to_review_meta_prefix = "github-devloop fix escalated to review-meta: ",
    review_meta_action_prefix = "github-devloop review-meta action: ",
    dependency_hold_prefix = "github-devloop dependency hold: ",
    intake_decision_prefix = "github-devloop intake decision: ",
    is_merging_pr_prefix = "github-devloop is merging PR #",
    merged_pr_prefix = "github-devloop merged PR #",
    no_fix_output = "(no fix output)",
    decomposed_prefix = "github-devloop decomposed blocked PR into ",
    decomposed_suffix = " follow-up issue(s)",
  },
  zh = {
    convergence_suffix = h("202d20e4b889e8a792e585b1e8af86e69caae8bebee68890efbc8ce6ada3e59ca8e694b6e7aa84"),
    narrowed_question_label = h("e694b6e7aa84e997aee9a298efbc9a"),
    angle_stances_label = h("e8a792e5baa6e7ab8be59cbaefbc9a"),
    verdict_summary_label = h("e4b889e8a792e7bb93e8aebaefbc9a"),
    comment_evidence_empty = h("efbc88e5a48de5aea1e8bdaee6aca1e8aeb0e5bd95e59ca8e788b620505220e8af84e8aebae4b8adefbc89"),
    thinking_started = h("6769746875622d6465766c6f6f7020e6809de88083efbc9ae585b1e8af86e5b7b2e5bc80e5a78b"),
    decision_prefix = h("6769746875622d6465766c6f6f7020e586b3e7ad96efbc9a"),
    convergence_round_prefix = h("6769746875622d6465766c6f6f7020e694b6e6959be8bdaee6aca120"),
    pr_review_convergence_round_prefix = h("6769746875622d6465766c6f6f7020505220e5a48de5aea1e694b6e6959be8bdaee6aca120"),
    reconcile_action_prefix = h("6769746875622d6465766c6f6f7020e8b083e5928ce58aa8e4bd9cefbc9a"),
    fix_reconcile_action_prefix = h("6769746875622d6465766c6f6f7020e4bfaee5a48de8b083e5928ce58aa8e4bd9cefbc9a"),
    review_reconcile_action_prefix = h("6769746875622d6465766c6f6f7020e5a48de5aea1e8b083e5928ce58aa8e4bd9cefbc9a"),
    reason_block_label = h("e58e9fe59ba0efbc9a"),
    reason_inline_label = h("e58e9fe59ba0efbc9a"),
    no_reason_provided = h("efbc88e69caae68f90e4be9be58e9fe59ba0efbc89"),
    implementation_started = h("6769746875622d6465766c6f6f7020e5ae9ee78eb0e5b7b2e5bc80e5a78b"),
    worktree_label = h("e5b7a5e4bd9ce6a091efbc9a"),
    branch_label = h("e58886e694afefbc9a"),
    head_label = h("e5a4b4e68f90e4baa4efbc9a"),
    base_branch_label = h("e59fbae58786e58886e694afefbc9a"),
    base_head_label = h("e59fbae58786e5a4b4e68f90e4baa4efbc9a"),
    implementation_failed_prefix = h("6769746875622d6465766c6f6f7020e5ae9ee78eb0e5a4b1e8b4a5efbc9a"),
    no_implementation_output = h("efbc88e697a0e5ae9ee78eb0e8be93e587baefbc89"),
    pr_opened_prefix = h("6769746875622d6465766c6f6f7020505220e5b7b2e68993e5bc80efbc9a23"),
    pr_ready_for_review = h("6769746875622d6465766c6f6f7020505220e5b7b2e58fafe5a48de5aea1"),
    pr_review_decision_prefix = h("6769746875622d6465766c6f6f7020505220e5a48de5aea1e586b3e7ad96efbc9a"),
    blocking_gap_label = h("e998bbe5a19ee7bcbae58fa3efbc9a"),
    merge_gate_failed_prefix = h("6769746875622d6465766c6f6f7020e59088e5b9b6e997a8e5a4b1e8b4a5efbc9a"),
    reproduce_locally_prefix = h("e8afb7e59ca8e4bb93e5ba93e6a0b9e79baee5bd95e794a82060"),
    reproduce_locally_suffix = h("6020e69cace59cb0e5a48de78eb0e38082"),
    fix_round_summary_label = h("e4bfaee5a48de8bdaee6aca1e69198e8a681efbc9a"),
    fix_pushed_for_rereview = h("6769746875622d6465766c6f6f7020e4bfaee5a48de5b7b2e68ea8e98081efbc8ce7ad89e5be85e5868de6aca1e5a48de5aea1"),
    previous_reviewed_head_label = h("e4b88ae4b880e8bdaee5a48de5aea1e5a4b4e68f90e4baa4efbc9a"),
    new_head_label = h("e696b0e5a4b4e68f90e4baa4efbc9a"),
    current_head_label = h("e5bd93e5898de5a4b4e68f90e4baa4efbc9a"),
    pr_head_advanced = h("6769746875622d6465766c6f6f7020505220e5a4b4e68f90e4baa4e59ca8e59088e5b9b6e689b9e58786e5908ee5898de8bf9befbc8ce9878de696b0e8bf9be585a5e5a48de5aea1"),
    fix_escalated_to_review_meta_prefix = h("6769746875622d6465766c6f6f7020e4bfaee5a48de58d87e7baa7e588b0207265766965772d6d657461efbc9a"),
    review_meta_action_prefix = h("6769746875622d6465766c6f6f70207265766965772d6d65746120e58aa8e4bd9cefbc9a"),
    dependency_hold_prefix = h("6769746875622d6465766c6f6f7020e4be9de8b596e69a82e5819cefbc9a"),
    intake_decision_prefix = h("6769746875622d6465766c6f6f7020e585a5e58fa3e586b3e7ad96efbc9a"),
    is_merging_pr_prefix = h("6769746875622d6465766c6f6f7020e6ada3e59ca8e59088e5b9b62050522023"),
    merged_pr_prefix = h("6769746875622d6465766c6f6f7020e5b7b2e59088e5b9b62050522023"),
    no_fix_output = h("efbc88e697a0e4bfaee5a48de8be93e587baefbc89"),
    decomposed_prefix = h("6769746875622d6465766c6f6f7020e5b7b2e5b086e998bbe5a19e20505220e68b86e58886e4b8ba20"),
    decomposed_suffix = h("20e4b8aae5908ee7bbad206973737565"),
  },
}

local human_comment_keys = {
  "convergence_suffix",
  "narrowed_question_label",
  "angle_stances_label",
  "verdict_summary_label",
  "comment_evidence_empty",
  "thinking_started",
  "decision_prefix",
  "convergence_round_prefix",
  "pr_review_convergence_round_prefix",
  "reconcile_action_prefix",
  "fix_reconcile_action_prefix",
  "review_reconcile_action_prefix",
  "reason_block_label",
  "reason_inline_label",
  "no_reason_provided",
  "implementation_started",
  "worktree_label",
  "branch_label",
  "head_label",
  "base_branch_label",
  "base_head_label",
  "implementation_failed_prefix",
  "no_implementation_output",
  "pr_opened_prefix",
  "pr_ready_for_review",
  "pr_review_decision_prefix",
  "blocking_gap_label",
  "merge_gate_failed_prefix",
  "reproduce_locally_prefix",
  "reproduce_locally_suffix",
  "fix_round_summary_label",
  "fix_pushed_for_rereview",
  "previous_reviewed_head_label",
  "new_head_label",
  "current_head_label",
  "pr_head_advanced",
  "fix_escalated_to_review_meta_prefix",
  "review_meta_action_prefix",
  "dependency_hold_prefix",
  "intake_decision_prefix",
  "is_merging_pr_prefix",
  "merged_pr_prefix",
  "no_fix_output",
  "decomposed_prefix",
  "decomposed_suffix",
}

local template_audit = {
  { id = "github-devloop-marker-comments", classification = "machine" },
  { id = "dedup-key-parts", classification = "machine" },
  { id = "state-labels", classification = "machine" },
  { id = "ai-sentinel", classification = "machine" },
  { id = "pr-title-and-body", classification = "repo-policy" },
  { id = "spec-amendment-issue-create", classification = "repo-policy" },
}

for _, key in ipairs(human_comment_keys) do
  table.insert(template_audit, { id = key, classification = "human" })
end

local configured_output_lang = nil

local function normalize_output_lang(value)
  local lang = tostring(value or ""):lower()
  if lang:match("^zh") then
    return "zh"
  end
  return "en"
end

function M.configure_output_lang(lang)
  configured_output_lang = lang and normalize_output_lang(lang) or nil
end

function M.output_lang(exec)
  if configured_output_lang ~= nil then
    return configured_output_lang
  end
  local ok, value = pcall(function()
    return M.read_env("FKST_OUTPUT_LANG", exec)
  end)
  if not ok then
    return "en"
  end
  return normalize_output_lang(value)
end

function M.comment_string(key, exec)
  local lang = M.output_lang(exec)
  local lang_strings = strings[lang] or strings.en
  return lang_strings[key] or strings.en[key] or tostring(key)
end

function M.comment_strings(lang)
  local normalized = normalize_output_lang(lang)
  return strings[normalized] or strings.en
end

function M.comment_template_audit()
  local copy = {}
  for _, row in ipairs(template_audit) do
    table.insert(copy, {
      id = row.id,
      classification = row.classification,
    })
  end
  return copy
end
end

return S
