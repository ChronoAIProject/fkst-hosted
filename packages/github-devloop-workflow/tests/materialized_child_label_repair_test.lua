local actions = require("core.materialize.actions")
local base_ids = require("devloop.base_ids")
local core = require("core")
local devloop_base = require("devloop.base")
local devloop_state = require("devloop.state")
local digest = require("core.digest")
local marker = require("core.marker")
local materialization = require("core.materialization")
local t = fkst.test

local repo = "owner/repo"
local origin_issue = 42
local child_issue = 108
local origin = base_ids.proposal_id(repo, origin_issue)

local function blueprint()
  return {
    schema = "fkst.workflow.v1",
    id = "repair-workflow",
    version = "1",
    summary = "Repair fixture.",
    applies_when = "The workflow is selected.",
    steps = {
      {
        id = "implement",
        title = "Implement",
        content = {
          kind = "static",
          intent = "Implement the requested change.",
        },
      },
    },
  }
end

local function generated_spec()
  return {
    title = "Implement the requested change",
    body = "Implement the requested change.",
  }
end

local function fixture_entry()
  return materialization.created_entry(
    origin,
    digest.blueprint_digest(blueprint()),
    blueprint().steps[1],
    materialization.EMPTY_PREDECESSOR_REF_DIGEST,
    generated_spec(),
    child_issue
  )
end

local function fixture_child(fields)
  local values = fields or {}
  local entry = fixture_entry()
  local lineage, lineage_err = marker.build_lineage_header(
    origin,
    digest.blueprint_digest(blueprint()),
    "implement"
  )
  t.is_nil(lineage_err)
  return {
    number = values.number or child_issue,
    title = values.title or generated_spec().title,
    body = values.body or (lineage
      .. "\n\n" .. generated_spec().body
      .. "\n\n<!-- fkst:github-proxy:issue-create:" .. entry.child_dedup .. " -->"),
    state = values.state or "OPEN",
    author_login = values.author_login or "app/fkst-test-bot",
    labels = values.labels or {},
    comments = values.comments or {},
  }
end

local function decision(entry, child, configured)
  devloop_base.configure_trusted_bot_login("fkst-test-bot")
  return actions.materialized_child_label_repair_decision(
    repo,
    origin,
    digest.blueprint_digest(blueprint()),
    blueprint(),
    entry,
    child,
    configured or { "fkst-dev", "fkst-security" }
  )
end

local function copy_entry()
  local copy = {}
  for key, value in pairs(fixture_entry()) do
    copy[key] = value
  end
  return copy
end

return {
  test_open_bot_child_with_exact_ledger_is_repairable = function()
    local result = decision(fixture_entry(), fixture_child())
    t.eq(result.action, "repair")
    t.eq(result.outcome, "applied(repaired-missing-work-label)")
  end,

  test_existing_target_label_is_idempotent_noop = function()
    local result = decision(fixture_entry(), fixture_child({ labels = { "bug", "fkst-dev" } }))
    t.eq(result.action, "noop")
    t.eq(result.outcome, "skip-idempotent(work-label-present)")
  end,

  test_other_exact_session_label_is_not_overwritten = function()
    local result = decision(fixture_entry(), fixture_child({ labels = { "fkst-security", "bug" } }))
    t.eq(result.action, "skip")
    t.eq(result.outcome, "skip-conflicting-work-label")
  end,

  test_closed_and_terminal_children_are_skipped = function()
    local closed = decision(fixture_entry(), fixture_child({ state = "CLOSED" }))
    t.eq(closed.outcome, "skip-child-closed")

    local labeled = decision(fixture_entry(), fixture_child({ labels = { "fkst-dev:impl-failed" } }))
    t.eq(labeled.outcome, "skip-child-terminal")

    local state_marker = devloop_state.state_marker(
      base_ids.proposal_id(repo, child_issue),
      "declined",
      "declined/2026-07-23T00-00-00Z"
    )
    local trusted = decision(fixture_entry(), fixture_child({
      comments = {
        { body = state_marker, author_login = "fkst-test-bot" },
      },
    }))
    t.eq(trusted.outcome, "skip-child-terminal")
  end,

  test_human_author_and_forged_lineage_are_skipped = function()
    local human = decision(fixture_entry(), fixture_child({ author_login = "human" }))
    t.eq(human.outcome, "skip-child-author-untrusted")

    local forged_lineage = marker.build_lineage_header(
      "github-devloop/issue/owner/repo/999",
      digest.blueprint_digest(blueprint()),
      "implement"
    )
    local forged = fixture_child({
      body = forged_lineage .. "\n\n" .. generated_spec().body
        .. "\n\n<!-- fkst:github-proxy:issue-create:" .. fixture_entry().child_dedup .. " -->",
    })
    local result = decision(fixture_entry(), forged)
    t.eq(result.outcome, "skip-child-lineage-untrusted")
  end,

  test_missing_stale_and_mismatched_ledgers_are_skipped = function()
    local missing = decision(nil, fixture_child())
    t.eq(missing.outcome, "skip-ledger-not-created")

    local stale = copy_entry()
    stale.blueprint_digest = "d-9999999999"
    t.eq(decision(stale, fixture_child()).outcome, "skip-ledger-stale")

    local wrong_dedup = copy_entry()
    wrong_dedup.child_dedup = wrong_dedup.child_dedup .. "/wrong"
    t.eq(decision(wrong_dedup, fixture_child()).outcome, "skip-ledger-identity-mismatch")

    local wrong_spec = copy_entry()
    wrong_spec.gen_spec_digest = "d-9999999999"
    t.eq(decision(wrong_spec, fixture_child()).outcome, "skip-child-ledger-mismatch")
  end,

  test_wrong_child_number_and_unconfigured_target_are_skipped = function()
    local wrong_number = decision(fixture_entry(), fixture_child({ number = 109 }))
    t.eq(wrong_number.outcome, "skip-child-number-mismatch")

    local unconfigured = decision(fixture_entry(), fixture_child(), { "fkst-security" })
    t.eq(unconfigured.outcome, "skip-target-work-label-unconfigured")
  end,

  test_repair_request_is_add_only_claimless_and_deterministic = function()
    devloop_base.configure_trusted_bot_login("fkst-test-bot")
    local first = actions.materialized_child_work_label_request(repo, origin, fixture_entry())
    local replay = actions.materialized_child_work_label_request(repo, origin, fixture_entry())
    t.eq(first.schema, "github-proxy.label.v1")
    t.eq(first.repo, repo)
    t.eq(first.issue_number, child_issue)
    t.eq(first.target_number, child_issue)
    t.eq(#first.add_labels, 1)
    t.eq(first.add_labels[1], "fkst-dev")
    t.eq(#first.remove_labels, 0)
    t.is_nil(first.claim)
    t.eq(first.dedup_key, replay.dedup_key)
    t.eq(first.source_ref.ref, repo .. "#issue/" .. tostring(child_issue))
  end,
}
