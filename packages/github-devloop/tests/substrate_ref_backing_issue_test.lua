local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local bump_head_sha = "dddddddddddddddddddddddddddddddddddddddd"

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

local function mock_backing_pr()
  entity_read_mocks.mock_pr_merge_view(t, {
    repo = "owner/repo",
    number = 27,
    head = "chore/substrate-ref-bump",
    head_sha = bump_head_sha,
    base_branch = "dev",
    state = "OPEN",
    comments = {},
    labels = {},
  })
  t.mock_command("gh pr diff", {
    stdout = ".fkst/substrate-ref\n",
    stderr = "",
    exit_code = 0,
  })
end

local function find_raise(raises, queue)
  for _, raised in ipairs(raises or {}) do
    if raised.queue == queue then
      return raised
    end
  end
  return nil
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
    mock_backing_pr()

    local result = run_judge(backing_candidate())

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "consensus.proposal"), nil)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
    local comment = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local reviewing = find_raise(result.raises, "devloop_reviewing")
    t.is_true(comment ~= nil)
    t.is_true(reviewing ~= nil)
    t.eq(comment.payload.pr_number, 27)
    t.is_true(comment.payload.body:find('proposal="github-devloop/issue/owner/repo/860"', 1, true) ~= nil)
    t.is_true(comment.payload.body:find('branch="chore/substrate-ref-bump"', 1, true) ~= nil)
    t.is_true(comment.payload.body:find('state="reviewing"', 1, true) ~= nil)
    t.eq(reviewing.payload.proposal_id, "github-devloop/issue/owner/repo/860")
    t.eq(reviewing.payload.pr_number, 27)
    t.eq(reviewing.payload.version, "substrate-ref-bump/" .. bump_head_sha)
    t.eq(reviewing.payload.source_ref.ref, "owner/repo#pr/27")
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
    mock_backing_pr()

    local result = h.run_implement(ready, opts("substrate-ref-backing-implement"))

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_open_request"), nil)
    local reviewing = find_raise(result.raises, "devloop_reviewing")
    t.is_true(reviewing ~= nil)
    t.eq(reviewing.payload.proposal_id, "github-devloop/issue/owner/repo/860")
    t.eq(reviewing.payload.pr_number, 27)
    t.eq(reviewing.payload.version, "substrate-ref-bump/" .. bump_head_sha)
  end,
}
