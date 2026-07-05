-- Regression: maybe_write_created_from_existing_child must be idempotent across
-- ticks. Once a slot's "created" ledger fact exists, the still-visible "generated"
-- fact for the same slot must NOT be re-derived into another "created" write that
-- returns true — doing so returns true on every 5m materialize tick and starves
-- frontier advancement (compute_frontier is never reached, so the next slot never
-- materializes and the multi-step cascade stalls after slot 1 merges).
--
-- Found by real supervise dogfood 2026-07-03: origin #89's merged scaffold child
-- #90 never advanced to the implement slot; the tick looped on
-- outcome=applied(parent-ledger-created) ~24 times instead of computing the
-- frontier. The harness never caught it because it stopped at child creation and
-- never ran the post-create ticks that loop here.
local actions = require("core.materialize.actions")
local base_ids = require("devloop.base_ids")
local core = require("core")
local t = fkst.test

local repo = "owner/repo"
local origin = base_ids.proposal_id(repo, 42)
local CHILD_DEDUP = "workflow/materialize/owner/repo/scaffold/d-0000000000"
local blueprint_fact = {
  origin = origin,
  workflow = "workflow-one",
  digest = "d-3588118930",
}
local record = {
  blueprint = {
    steps = {
      {
        id = "scaffold",
        content = {
          kind = "static",
          intent = "Scaffold the implementation.",
        },
      },
    },
  },
}

local function generated_fact()
  return {
    state = "generated",
    origin = origin,
    blueprint_digest = "d-3588118930",
    slot = "scaffold",
    predecessor_ref_digest = "d-0000000000",
    gen_contract_digest = "d-2364386957",
    gen_spec_digest = "d-0975672535",
    child_dedup = CHILD_DEDUP,
  }
end

local function created_fact()
  local f = generated_fact()
  f.state = "created"
  return f
end

local function comment(body)
  return { body = body }
end

local function issue_created_comment(child_dedup, child_issue)
  return comment('<!-- fkst:github-proxy:issue-created:v1 dedup="' .. child_dedup .. '" issue="' .. tostring(child_issue) .. '" -->')
end

-- The reconcile flow passes a trusted-comment filter; the test controls the
-- comments directly, so a passthrough is faithful to the function under test.
local function trusted_passthrough(_core, comments)
  return comments or {}
end

local function noop_log() end

return {
  -- THE FIX: a slot that already has a "created" fact must be skipped, so the
  -- function returns false and the reconcile flow can advance to compute_frontier.
  test_skips_generated_when_created_fact_already_exists = function()
    local facts = { generated_fact(), created_fact() }
    local current = { comments = { issue_created_comment(CHILD_DEDUP, 90) } }
    local wrote = actions.maybe_write_created_from_existing_child(
      core, {}, repo, 42, origin, blueprint_fact, record, facts, current, trusted_passthrough, noop_log
    )
    t.is_true(not wrote)
  end,

  -- (The first-time write path — a generated fact whose child is visible with no
  -- created fact yet, which raises the created-ledger comment request — is covered
  -- end-to-end by materialize_reconcile_test in a full raise-capable context; a
  -- single-package unit test cannot raise the cross-package comment queue.)

  -- A generated fact whose child issue is not yet visible returns false (nothing
  -- to record) and does not spuriously claim a write.
  test_no_write_when_child_not_visible = function()
    local facts = { generated_fact() }
    local current = { comments = {} }
    local wrote = actions.maybe_write_created_from_existing_child(core, {
      search_created_issue = function()
        return nil
      end,
    }, repo, 42, origin, blueprint_fact, record, facts, current, trusted_passthrough, noop_log)
    t.is_true(not wrote)
  end,
}
