local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core

local function assert_language_preamble(prompt)
  t.is_true(prompt:find("Write all output in English; quote code identifiers and cited originals verbatim.", 1, true) ~= nil)
end

local function assert_actor_preamble_slots(prompt)
  assert_language_preamble(prompt)
  t.is_true(prompt:find("Before acting, identify the established theory or industry best practice governing this change", 1, true) ~= nil)
  t.is_true(prompt:find("surface that blocker explicitly instead of silently improvising or claiming success", 1, true) ~= nil)
  t.is_nil(prompt:find("grounds for rejection or narrowing", 1, true))
end

local function conflict(extra)
  local payload = {
    schema = "github-devloop.v1",
    repo = "owner/repo",
    upstream_branch = "dev",
    integration_branch = "integration/dev",
    upstream_sha = "aaaa1111",
    integration_sha = "bbbb2222",
    dedup_key = core.branch_sync_dedup_key("owner/repo", "dev", "integration/dev", "aaaa1111"),
    source_ref = core.branch_sync_source_ref("owner/repo", "dev", "integration/dev"),
  }
  for key, value in pairs(extra or {}) do
    payload[key] = value
  end
  return payload
end

return {
  test_integration_package_installs_only_sync_conflict_prompt_role = function()
    t.eq(type(core.build_sync_conflict_prompt), "function")
    t.is_nil(core.build_implement_prompt)
    t.is_nil(core.build_fix_prompt)
    t.is_nil(core.build_intake_prompt)
    t.is_nil(core.build_decompose_prompt)
    t.is_nil(core.build_review_meta_prompt)
    t.is_nil(core.parse_intake_action)
    t.is_nil(core.parse_review_meta_action)
  end,

  test_sync_conflict_fingerprint_uses_stable_identity_and_paths = function()
    local one = core.sync_conflict_fingerprint(conflict(), table.concat({
      "100644 abc 1\tpackages/github-devloop/core.lua",
      "100644 def 2\tpackages/github-devloop/core.lua",
      "",
    }, "\n"))
    local two = core.sync_conflict_fingerprint(conflict(), table.concat({
      "100644 xxx 1\tpackages/github-devloop/core.lua",
      "100644 yyy 2\tpackages/github-devloop/core.lua",
      "",
    }, "\n"))
    local changed = core.sync_conflict_fingerprint(conflict({ upstream_sha = "cccc3333" }), table.concat({
      "100644 abc 1\tpackages/github-devloop/core.lua",
      "",
    }, "\n"))

    t.eq(one, two)
    t.is_true(one ~= changed)
  end,

  test_sync_conflict_attempt_count_round_trips_cache = function()
    local item = conflict()
    local fingerprint = core.sync_conflict_fingerprint(item, "100644 abc 1\tcore.lua\n")
    t.eq(core.sync_conflict_attempt_count(item, fingerprint), 0)
    core.record_sync_conflict_attempt(item, fingerprint, 2)
    t.eq(core.sync_conflict_attempt_count(item, fingerprint), 2)
  end,

  test_sync_conflict_escalation_request_carries_terminal_why = function()
    local item = conflict()
    local fingerprint = core.sync_conflict_fingerprint(item, "100644 abc 1\tcore.lua\n")
    local request = core.build_sync_conflict_escalation_request(
      item,
      fingerprint,
      core.max_sync_conflict_attempts(),
      "sync conflict remains unresolved after codex completed",
      "100644 abc 1\tcore.lua\n"
    )

    t.eq(request.schema, "github-proxy.issue-create.v1")
    t.eq(request.repo, "owner/repo")
    t.is_true(request.title:find("Branch sync conflict requires manual resolution", 1, true) ~= nil)
    t.is_true(request.body:find("Reason: sync conflict remains unresolved after codex completed", 1, true) ~= nil)
    t.is_true(request.body:find("Attempt: 3", 1, true) ~= nil)
    t.is_true(request.body:find("Fingerprint: " .. fingerprint, 1, true) ~= nil)
    t.is_true(request.body:find("- core.lua", 1, true) ~= nil)
    t.eq(request.source_ref.ref, "owner/repo#branch-sync/dev/integration/dev")
  end,

  test_sync_conflict_prompt_omits_issue_pr_history_directive = function()
    local prompt = core.build_sync_conflict_prompt({
      repo = "owner/repo",
      upstream_branch = "dev",
      integration_branch = "integration/dev",
      upstream_sha = "abcdef123456",
      integration_sha = "123456abcdef",
    })

    assert_actor_preamble_slots(prompt)
    t.is_nil(prompt:find("COMPLETE GitHub comment stream of the subject issue/PR", 1, true))
    t.is_nil(prompt:find("gh issue view --comments / gh pr view --comments", 1, true))
    t.is_nil(prompt:find("{{", 1, true))
  end,
}
