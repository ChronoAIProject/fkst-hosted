local t = fkst.test

-- The auto-refinement budget is what stops a convergence terminal from being a
-- full stop for a self-driving session -- and, equally, what stops it becoming an
-- infinite retry. Both directions are pinned here.

local function marker(proposal_id, round)
  return {
    body = "fkst: reintake\n\n<!-- fkst:github-devloop:auto-refine:v1 proposal=\""
      .. proposal_id .. "\" round=\"" .. tostring(round) .. "\" cause=\"no-semantic-progress\" -->",
  }
end

return {
  test_exhausted_budget_hands_off_a_forward_decompose_payload = function()
    -- When the budget is spent, reconcile raises the decompose queue instead of
    -- leaving the row to idle out its 1410-minute operator wait. That handoff is
    -- only real if the payload is the FORWARD shape the decompose department
    -- accepts -- a replay-shaped or review-bound payload would be dropped and the
    -- row would silently sit for a day anyway.
    local builders = require("devloop.payloads.builders")
    local decompose = require("devloop.decompose")
    local base_ids = require("devloop.base_ids")

    local payload = builders.build_devloop_decompose_payload({
      proposal_id = "github-devloop/issue/acme/site/42",
      issue_version = "github-devloop/issue/acme/site/42/intake/7",
      round = 1,
      source_ref = base_ids.issue_source_ref("acme/site", 42),
    })

    t.is_true(decompose.is_supported_decompose(payload))
    t.eq(payload.schema, "github-devloop.decompose.v1")
    t.eq(payload.version, "github-devloop/issue/acme/site/42/intake/7")
    -- Forward, not replay: a review binding or child counts would change the dedup
    -- key and route this down the replay path.
    t.eq(payload.review_proposal_id, nil)
    t.eq(payload.head_sha, nil)
    t.eq(payload.expected_child_count, nil)
    t.eq(payload.completed_child_count, nil)
  end,

  test_every_terminal_cause_is_refinable_and_nothing_else_is = function()
    local rounds = require("devloop.convergence.rounds")

    t.eq(rounds.is_refinable_cause("evidence-continuation-budget-exhausted"), true)
    t.eq(rounds.is_refinable_cause("no-semantic-progress"), true)
    -- external-evidence-required included on purpose: unattended, "we need a fact
    -- we do not have" is a request for a decision, and a refinement can record one.
    -- The budget, not the cause, is what stops the loop.
    t.eq(rounds.is_refinable_cause("external-evidence-required"), true)
    -- A non-terminal string must never open the refine path.
    t.eq(rounds.is_refinable_cause("something-else"), false)
    t.eq(rounds.is_refinable_cause(nil), false)
  end,

  test_budget_is_counted_from_durable_markers_not_memory = function()
    local rounds = require("devloop.convergence.rounds")
    local pid = "github-devloop/issue/acme/site/42"

    t.eq(rounds.auto_refine_count({}, pid), 0)
    t.eq(rounds.auto_refine_budget_remaining({}, pid), true)

    t.eq(rounds.auto_refine_count({ marker(pid, 1) }, pid), 1)
    t.eq(rounds.auto_refine_budget_remaining({ marker(pid, 1) }, pid), true)

    -- At the cap the loop must hand over to a human rather than take another lap.
    local spent = { marker(pid, 1), marker(pid, 2) }
    t.eq(rounds.auto_refine_count(spent, pid), rounds.MAX_AUTO_REFINEMENTS)
    t.eq(rounds.auto_refine_budget_remaining(spent, pid), false)
  end,

  test_another_proposals_refinements_do_not_spend_this_budget = function()
    -- One issue's laps must never silently consume a sibling's, or a busy repo
    -- would starve items it never even reviewed.
    local rounds = require("devloop.convergence.rounds")
    local mine = "github-devloop/issue/acme/site/42"
    local theirs = "github-devloop/issue/acme/site/99"

    local comments = { marker(theirs, 1), marker(theirs, 2) }
    t.eq(rounds.auto_refine_count(comments, mine), 0)
    t.eq(rounds.auto_refine_budget_remaining(comments, mine), true)
  end,

  test_auto_refine_uses_an_action_the_marker_grammar_already_accepts = function()
    -- The reconcile marker validates its action against drop/re-design/re-cluster
    -- and ERRORS otherwise -- before the CAS decision is logged, so an unsupported
    -- action does not merely mislabel the outcome, it aborts the whole reconcile
    -- pass. Auto-refinement reuses `re-design` rather than inventing a verb.
    local conv_reconcile = require("devloop.convergence.reconcile")
    local pid = "github-devloop/issue/acme/site/42"

    local m = conv_reconcile.reconcile_marker(pid, "v1", 1, "re-design", "no-semantic-progress")
    t.is_true(m:find("re-design", 1, true) ~= nil)

    local ok = pcall(conv_reconcile.reconcile_marker, pid, "v1", 1, "refine", "no-semantic-progress")
    t.eq(ok, false)
  end,

  test_marker_carries_proposal_round_and_cause = function()
    local rounds = require("devloop.convergence.rounds")
    local m = rounds.auto_refine_marker("github-devloop/issue/acme/site/42", 2, "no-semantic-progress")

    t.is_true(m:find("fkst:github%-devloop:auto%-refine:v1") ~= nil)
    t.is_true(m:find("github-devloop/issue/acme/site/42", 1, true) ~= nil)
    t.is_true(m:find('round="2"', 1, true) ~= nil)
    t.is_true(m:find('cause="no-semantic-progress"', 1, true) ~= nil)
    -- Round-trips through its own counter.
    t.eq(rounds.auto_refine_count({ { body = m } }, "github-devloop/issue/acme/site/42"), 1)
  end,
}
