-- parse_workflow_selection must tolerate a reasoning model's preamble lines and
-- still extract the unique ⟦FKST:WORKFLOW_SELECT⟧ sentinel. Before this, the parser
-- demanded the whole codex output be exactly one line, so any verbose model (e.g.
-- gpt-5.5, which prepends reasoning) came back "none-or-unparseable" and repo-local
-- workflow routing silently never fired — a triggered custom workflow fell through
-- to the plain devloop. Uniqueness of the sentinel stays the injection guard.
local workflow_select = require("workflow_select")
local t = fkst.test

local MARK = "⟦FKST:WORKFLOW_SELECT⟧"

return {
  test_parses_clean_single_line = function()
    t.eq(
      workflow_select.parse_workflow_selection(MARK .. " hr-employee-onboarding"),
      "hr-employee-onboarding"
    )
  end,

  test_parses_bare_single_line_id_without_sentinel = function()
    t.eq(
      workflow_select.parse_workflow_selection("hr-employee-onboarding"),
      "hr-employee-onboarding"
    )
  end,

  test_parses_sentinel_after_reasoning_preamble = function()
    -- The regression case: reasoning lines precede the sentinel.
    local out = "Let me consider the request.\n"
      .. "This issue is an onboarding request that matches the HR workflow.\n"
      .. MARK
      .. " hr-employee-onboarding"
    t.eq(workflow_select.parse_workflow_selection(out), "hr-employee-onboarding")
  end,

  test_none_sentinel_after_preamble_is_nil = function()
    local out = "No catalog workflow fits this issue.\n" .. MARK .. " none"
    t.is_true(workflow_select.parse_workflow_selection(out) == nil)
  end,

  test_two_sentinels_is_ambiguous_nil = function()
    -- Injection guard: an echoed or injected second sentinel must not smuggle a
    -- selection through. Ambiguity is rejected.
    local out = MARK .. " workflow-alpha\n" .. MARK .. " workflow-beta"
    t.is_true(workflow_select.parse_workflow_selection(out) == nil)
  end,

  test_empty_output_is_nil = function()
    t.is_true(workflow_select.parse_workflow_selection("") == nil)
  end,
}
