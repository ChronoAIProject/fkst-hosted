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
  test_only_causes_a_rewrite_can_actually_fix_are_refinable = function()
    local rounds = require("devloop.convergence.rounds")

    t.eq(rounds.is_refinable_cause("evidence-continuation-budget-exhausted"), true)
    t.eq(rounds.is_refinable_cause("no-semantic-progress"), true)
    -- external-evidence-required needs a fact from OUTSIDE the issue; amending the
    -- spec cannot manufacture it, so retrying would bury the real request.
    t.eq(rounds.is_refinable_cause("external-evidence-required"), false)
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
