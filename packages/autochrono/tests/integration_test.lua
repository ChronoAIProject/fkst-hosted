local t = fkst.test
local core = require("core")
local propose_mapping = require("departments.propose.mapping")

local function nonce()
  return tostring({}):gsub("[^%w._-]", "_")
end

local function runtime_root(name)
  return "/tmp/fkst-packages-test/autochrono/" .. tostring(now()) .. "/" .. nonce() .. "/" .. name
end

local function opts(name)
  return {
    env = {
      FKST_RUNTIME_ROOT = runtime_root(name),
    },
  }
end

local function issue(extra)
  local value = {
    schema = "autochrono.issue.v1",
    repo = "owner/repo",
    issue_number = 42,
    title = "Bridge issue",
    url = "https://github.example/owner/repo/issues/42",
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    source_ref = {
      kind = "external",
      ref = "owner/repo#issue/42",
    },
    dedup_key = "owner/repo#issue#42@2026-06-03T01:02:03Z",
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function consensus_reached(extra)
  local value = {
    schema = "consensus.consensus_reached.v1",
    proposal_id = "autochrono/issue/owner/repo/42",
    decision = "approve",
    body = "Thanks for opening this. I will review the details and follow up with the next concrete step.",
    dedup_key = "consensus:autochrono/issue/owner/repo/42/2026-06-03T01-02-03Z",
    source_ref = {
      kind = "external",
      ref = "owner/repo#issue/42",
    },
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function run_reply(event_payload, run_opts)
  return t.run_department("departments/reply/main.lua", {
    queue = "consensus.consensus_reached",
    payload = event_payload,
  }, run_opts)
end

local function run_propose(event_payload, run_opts)
  return t.run_department("departments/propose/main.lua", {
    queue = "issue",
    payload = event_payload,
  }, run_opts)
end

local function codex_calls()
  local calls = {}
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find("codex exec", 1, true) ~= nil then
      table.insert(calls, call)
    end
  end
  return calls
end

return {
  test_propose_open_issue_raises_consensus_proposal = function()
    t.mock_command("codex exec", { stdout = "should not be used", exit_code = 0 })

    local payload = propose_mapping.build_proposal(issue())
    t.eq(payload.schema, "consensus.proposal.v1")
    t.eq(payload.proposal_id, "autochrono/issue/owner/repo/42")
    t.eq(payload.dedup_key, "autochrono/issue/owner/repo/42/2026-06-03T01-02-03Z")
    t.eq(payload.proposal_id:find(":", 1, true), nil)
    t.eq(payload.proposal_id:find("@", 1, true), nil)
    t.eq(payload.source_ref.kind, "external")
    t.eq(payload.source_ref.ref, "owner/repo#issue/42")
    t.is_true(#payload.content_fetch <= 4000)
    t.is_true(payload.content_fetch:find("source_ref owner/repo#issue/42", 1, true) ~= nil)
    t.is_true(payload.content_fetch:find("full issue body", 1, true) ~= nil)
    t.is_true(payload.content_fetch:find("ALL comments", 1, true) ~= nil)
    t.is_true(payload.content_fetch:find("Body above is only a brief", 1, true) ~= nil)
    t.is_true(payload.body:find("Repository: owner/repo", 1, true) ~= nil)
    t.is_true(payload.body:find("Number: 42", 1, true) ~= nil)
    t.is_true(payload.body:find("Title: Bridge issue", 1, true) ~= nil)
    t.is_true(payload.body:find("URL: https://github.example/owner/repo/issues/42", 1, true) ~= nil)
    t.is_true(payload.body:find("Updated at: 2026-06-03T01:02:03Z", 1, true) ~= nil)
    t.is_nil(payload.body:find("{{", 1, true))
    t.eq(#codex_calls(), 0)
  end,

  test_propose_skips_closed_and_unsupported_schema = function()
    t.eq(core.is_eligible(issue({ state = "CLOSED" })), false)
    t.eq(core.is_eligible(issue({ schema = "other.issue.v1" })), false)
    t.eq(#codex_calls(), 0)
  end,

  test_propose_cache_versions_by_updated_at = function()
    t.mock_command("codex exec", { stdout = "should not be used", exit_code = 0 })
    local first_key = core.proposal_cache_key("owner/repo", 42, "2026-06-03T01:02:03Z")
    local second_key = core.proposal_cache_key("owner/repo", 42, "2026-06-03T01:02:03Z")
    local changed_key = core.proposal_cache_key("owner/repo", 42, "2026-06-04T05:06:07Z")
    local changed_payload = propose_mapping.build_proposal(issue({ updated_at = "2026-06-04T05:06:07Z" }))

    t.eq(first_key, second_key)
    t.is_true(first_key ~= changed_key)
    t.eq(changed_payload.dedup_key, "autochrono/issue/owner/repo/42/2026-06-04T05-06-07Z")
    t.eq(#codex_calls(), 0)
  end,

  test_propose_open_issue_records_consensus_proposal = function()
    t.mock_command("codex exec", { stdout = "should not be used", exit_code = 0 })

    local result = run_propose(issue(), opts("propose-open"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "consensus.proposal")

    local payload = result.raises[1].payload
    local proposal_id = "autochrono/issue/owner/repo/42"
    t.eq(payload.schema, "consensus.proposal.v1")
    t.eq(payload.proposal_id, proposal_id)
    t.is_true(payload.dedup_key:find(proposal_id, 1, true) == 1)
    t.eq(payload.source_ref.kind, "external")
    t.eq(payload.source_ref.ref, "owner/repo#issue/42")
    t.is_true(#payload.content_fetch <= 4000)
    t.is_true(payload.content_fetch:find("source_ref owner/repo#issue/42", 1, true) ~= nil)
    t.is_true(payload.content_fetch:find("full issue body", 1, true) ~= nil)
    t.is_true(payload.content_fetch:find("ALL comments", 1, true) ~= nil)
    t.is_true(payload.content_fetch:find("Body above is only a brief", 1, true) ~= nil)
    t.eq(#codex_calls(), 0)
  end,

  test_propose_skips_closed_issue_end_to_end = function()
    t.mock_command("codex exec", { stdout = "should not be used", exit_code = 0 })

    local result = run_propose(issue({ state = "CLOSED" }), opts("propose-closed"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(#codex_calls(), 0)
  end,

  test_propose_skips_unsupported_schema_end_to_end = function()
    t.mock_command("codex exec", { stdout = "should not be used", exit_code = 0 })

    local result = run_propose(issue({ schema = "other.issue.v1" }), opts("propose-schema"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(#codex_calls(), 0)
  end,

  test_propose_version_idempotency_end_to_end = function()
    t.mock_command("codex exec", { stdout = "should not be used", exit_code = 0 })
    local run_opts = opts("propose-version")

    local first = run_propose(issue(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 1)

    local second = run_propose(issue(), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 0)

    local third = run_propose(issue({ updated_at = "2026-06-04T05:06:07Z" }), run_opts)
    t.eq(third.exit_code, 0)
    t.eq(#third.raises, 1)
    t.eq(#codex_calls(), 0)
  end,

  test_propose_fail_closed_on_oversized_issue = function()
    t.mock_command("codex exec", { stdout = "should not be used", exit_code = 0 })

    local result = run_propose(issue({ title = string.rep("x", 241) }), opts("propose-oversized"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(#codex_calls(), 0)
  end,

  test_propose_skips_unsafe_issue_ref_end_to_end = function()
    t.mock_command("codex exec", { stdout = "should not be used", exit_code = 0 })

    local unsafe = run_propose(issue({ repo = "owner:repo" }), opts("propose-unsafe-ref"))
    t.eq(unsafe.exit_code, 0)
    t.eq(#unsafe.raises, 0)

    local oversized = run_propose(issue({ issue_number = string.rep("7", 31) }), opts("propose-oversized-ref"))
    t.eq(oversized.exit_code, 0)
    t.eq(#oversized.raises, 0)
    t.eq(#codex_calls(), 0)
  end,

  test_reply_approve_raises_autochrono_reply = function()
    t.mock_command("codex exec", { stdout = "should not be used", exit_code = 0 })

    local reached = consensus_reached()
    local result = run_reply(reached, opts("reply-approve"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "reply")

    local payload = result.raises[1].payload
    t.eq(payload.schema, "autochrono.reply.v1")
    t.eq(payload.repo, "owner/repo")
    t.eq(payload.issue_number, "42")
    t.eq(payload.body, reached.body)
    t.eq(payload.dedup_key, "autochrono:owner/repo#issue/42")
    t.eq(payload.source_ref.kind, "external")
    t.eq(payload.source_ref.ref, "owner/repo#issue/42")
    t.eq(#codex_calls(), 0)
  end,

  test_reply_skips_reject_and_foreign_proposal = function()
    local rejected = run_reply(consensus_reached({ decision = "reject" }), opts("reply-reject"))
    t.eq(rejected.exit_code, 0)
    t.eq(#rejected.raises, 0)

    local foreign = run_reply(consensus_reached({ proposal_id = "other/issue/owner/repo/42" }), opts("reply-foreign"))
    t.eq(foreign.exit_code, 0)
    t.eq(#foreign.raises, 0)
    t.eq(#codex_calls(), 0)
  end,

  test_reply_cache_skips_second_approve_for_issue = function()
    t.mock_command("codex exec", { stdout = "should not be used", exit_code = 0 })
    local run_opts = opts("reply-cache")

    local first = run_reply(consensus_reached(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 1)

    local second = run_reply(consensus_reached({ body = "Different approved body." }), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 0)
    t.eq(#codex_calls(), 0)
  end,

  test_reply_skips_malformed_reached_without_marking_replied = function()
    local run_opts = opts("reply-malformed")

    -- approve but empty body -> no reply raised
    local bad = run_reply(consensus_reached({ body = "" }), run_opts)
    t.eq(bad.exit_code, 0)
    t.eq(#bad.raises, 0)

    -- the malformed event must NOT have marked the issue replied: a later well-formed
    -- approve for the same issue still produces a reply
    local good = run_reply(consensus_reached(), run_opts)
    t.eq(good.exit_code, 0)
    t.eq(#good.raises, 1)
    t.eq(good.raises[1].payload.body, consensus_reached().body)
    t.eq(#codex_calls(), 0)
  end,
}
