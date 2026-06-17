local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local function substrate_ref_backing_body(pr_number)
  return "Backing issue for the autonomous fkst-substrate pin bump PR. PR: #"
    .. tostring(pr_number or 27)
    .. ".\n\nThis exists only to authorize the existing `.fkst/substrate-ref` bump lifecycle."
end

local function backing_candidate()
  return core.build_devloop_intake_candidate_payload("owner/repo", "860", "2026-06-03T01:02:03Z")
end

local function mock_intake_judge_backing_issue()
  entity_read_mocks.mock_issue_view_selector(t, {
    number = 860,
    title = "chore: bump fkst-substrate pin",
    body = substrate_ref_backing_body(27),
    labels = {},
    comments = {},
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "title,body,updatedAt,labels,comments,state,assignees,author", 1)
end

local function run_judge(payload)
  return t.run_department("departments/intake_judge/main.lua", {
    queue = "devloop_intake_candidate",
    payload = payload,
  }, opts("substrate-ref-backing-intake"))
end

return {
  test_core_identifies_substrate_ref_backing_issue = function()
    t.eq(core.substrate_ref_backing_issue_pr_number({
      title = "chore: bump fkst-substrate pin",
      body = substrate_ref_backing_body(27),
    }), 27)
    t.eq(core.is_substrate_ref_backing_issue({
      title = "chore: bump fkst-substrate pin",
      body = substrate_ref_backing_body(27),
    }), true)
    t.eq(core.is_substrate_ref_backing_issue({
      title = "chore: bump fkst-substrate pin",
      body = "Please implement a normal package feature.",
    }), false)
  end,

  test_intake_judge_skips_substrate_ref_backing_issue_without_codex_or_consensus = function()
    h.mock_bot_env()
    mock_intake_judge_backing_issue()

    local result = run_judge(backing_candidate())

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(h.count_calls("codex exec"), 0)
  end,

  test_implement_skips_substrate_ref_backing_issue_without_codex_or_pr = function()
    local event = h.reached({
      proposal_id = "github-devloop/issue/owner/repo/860",
      source_ref = {
        kind = "external",
        ref = "owner/repo#issue/860",
      },
    })
    local ready = core.build_devloop_ready_payload(event)
    h.mock_issue_implement_raw({ "fkst-dev:ready" }, {
      core.state_marker(event.proposal_id, "ready", ready.dedup_key),
    }, {
      number = 860,
      title = "chore: bump fkst-substrate pin",
      body = substrate_ref_backing_body(27),
    })

    local result = h.run_implement(ready, opts("substrate-ref-backing-implement"))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(h.count_calls("codex exec"), 0)
  end,
}
