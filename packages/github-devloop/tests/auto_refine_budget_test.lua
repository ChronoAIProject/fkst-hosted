local t = fkst.test

-- The auto-refinement budget is what stops a convergence terminal from being a
-- full stop for a self-driving session -- and, equally, what stops it becoming an
-- infinite retry. Both directions are pinned here.

local core = require("core")

-- A marker the LOOP wrote. author_login matters: the budget is counted from
-- trusted comments only, so a fixture without it is not a spent refinement.
local function marker(proposal_id, round, author)
  return {
    author_login = author or core._test_bot_login,
    body = "fkst: reintake\n\n<!-- fkst:github-devloop:auto-refine:v1 proposal=\""
      .. proposal_id .. "\" round=\"" .. tostring(round) .. "\" cause=\"no-semantic-progress\" -->",
  }
end

return {
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
    t.eq(rounds.auto_refine_budget_remaining({}, pid, 2), true)

    t.eq(rounds.auto_refine_count({ marker(pid, 1) }, pid), 1)
    t.eq(rounds.auto_refine_budget_remaining({ marker(pid, 1) }, pid, 2), true)

    -- At the cap the loop must hand over to a human rather than take another lap.
    local spent = { marker(pid, 1), marker(pid, 2) }
    t.eq(rounds.auto_refine_count(spent, pid), 2)
    t.eq(rounds.auto_refine_budget_remaining(spent, pid, 2), false)
  end,

  test_refinement_is_on_by_default_and_a_session_can_switch_it_off = function()
    -- The default is what a session gets when its trigger says nothing. It is ON,
    -- so an unattended deployment keeps amending rather than waiting for a human.
    local rounds = require("devloop.convergence.rounds")
    local pid = "github-devloop/issue/acme/site/42"

    t.eq(rounds.DEFAULT_MAX_AUTO_REFINEMENTS, 100)
    t.eq(rounds.auto_refine_budget_remaining({}, pid, rounds.DEFAULT_MAX_AUTO_REFINEMENTS), true)
    -- An explicit 0 is how a session asks for the stop-for-a-human behaviour.
    t.eq(rounds.auto_refine_budget_remaining({}, pid, 0), false)
    -- An unresolvable budget still reads as the default rather than "unlimited",
    -- so a bad value can never remove the bound entirely.
    t.eq(rounds.auto_refine_budget_remaining({}, pid, nil), true)
  end,

  test_a_configured_budget_is_bounded_and_typo_tolerant = function()
    local rounds = require("devloop.convergence.rounds")
    local pid = "github-devloop/issue/acme/site/42"

    -- A budget only bounds if it is itself bounded.
    t.eq(rounds.auto_refine_budget_remaining({ marker(pid, 1) }, pid, 1), false)
    t.eq(rounds.auto_refine_budget_remaining({ marker(pid, 1) }, pid, 5), true)
    -- The ceiling matches the default so a session can request the full budget.
    t.eq(rounds.MAX_AUTO_REFINEMENTS_CEILING, rounds.DEFAULT_MAX_AUTO_REFINEMENTS)
  end,

  test_another_proposals_refinements_do_not_spend_this_budget = function()
    -- One issue's laps must never silently consume a sibling's, or a busy repo
    -- would starve items it never even reviewed.
    local rounds = require("devloop.convergence.rounds")
    local mine = "github-devloop/issue/acme/site/42"
    local theirs = "github-devloop/issue/acme/site/99"

    local comments = { marker(theirs, 1), marker(theirs, 2) }
    t.eq(rounds.auto_refine_count(comments, mine), 0)
    t.eq(rounds.auto_refine_budget_remaining(comments, mine, 2), true)
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
    t.eq(
      rounds.auto_refine_count(
        { { author_login = core._test_bot_login, body = m } },
        "github-devloop/issue/acme/site/42"
      ),
      1
    )
  end,

  test_a_user_cannot_burn_the_budget_by_pasting_the_marker = function()
    -- The marker text is public: it is visible in every refinement comment. If the
    -- count were taken over all comments, anyone able to comment on the issue could
    -- paste it twice and silently switch self-refinement back off for a session that
    -- explicitly opted in -- with nothing to say it had happened.
    local rounds = require("devloop.convergence.rounds")
    local pid = "github-devloop/issue/acme/site/42"

    local forged = { marker(pid, 1, "mallory"), marker(pid, 2, "mallory") }
    t.eq(rounds.auto_refine_count(forged, pid), 0)
    t.eq(rounds.auto_refine_budget_remaining(forged, pid, 2), true)

    -- The loop's own marker still counts, so the budget is a real bound.
    local genuine = { marker(pid, 1) }
    t.eq(rounds.auto_refine_count(genuine, pid), 1)

    -- And a forged one cannot pad a genuine one to exhaust the budget.
    local mixed = { marker(pid, 1), marker(pid, 2, "mallory") }
    t.eq(rounds.auto_refine_count(mixed, pid), 1)
    t.eq(rounds.auto_refine_budget_remaining(mixed, pid, 2), true)
  end,
}
