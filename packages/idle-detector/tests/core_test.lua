local core = require("core")
local claim_identity = require("forge.github.claim_identity")
local t = fkst.test

local function identity()
  local value, err = claim_identity.from_values("owner/repo", "fkst-test-bot[bot]")
  if err ~= nil then
    error(err.why or "unexpected identity error", 0)
  end
  return value
end

return {
  test_claim_identity_normalizes_repo_and_bot_login = function()
    local value, err = claim_identity.from_values(" owner/repo ", " fkst-test-bot[bot] ")

    t.is_nil(err)
    t.eq(value.repo, "owner/repo")
    t.eq(value.bot_login, "fkst-test-bot")
    t.eq(value.source_ref.kind, "github-assignee-query")
    t.eq(value.source_ref.ref, "owner/repo#issues?state=open&assignee=fkst-test-bot")
  end,

  test_claim_identity_fails_closed_on_missing_or_malformed_scope = function()
    for _, case in ipairs({
      { repo = "", bot = "fkst-test-bot", why = "missing FKST_GITHUB_REPO" },
      { repo = "owner", bot = "fkst-test-bot", why = "malformed FKST_GITHUB_REPO" },
      { repo = "owner/repo", bot = "", why = "missing FKST_GITHUB_BOT_LOGIN" },
      { repo = "owner/repo", bot = "bad login", why = "malformed FKST_GITHUB_BOT_LOGIN" },
    }) do
      local value, err = claim_identity.from_values(case.repo, case.bot)
      t.is_nil(value)
      t.eq(err.error_class, "github-claim-identity-unverified")
      t.is_true(err.why:find(case.why, 1, true) ~= nil)
    end
  end,

  test_claim_identity_is_shared_forge_boundary_not_idle_local = function()
    local value, err = claim_identity.read(function(name)
      if name == "FKST_GITHUB_REPO" then
        return "owner/repo"
      end
      if name == "FKST_GITHUB_BOT_LOGIN" then
        return "fkst-test-bot[bot]"
      end
      return nil
    end)

    t.is_nil(err)
    t.eq(value.repo, "owner/repo")
    t.eq(value.bot_login, "fkst-test-bot")
    t.eq(value.source_ref.ref, "owner/repo#issues?state=open&assignee=fkst-test-bot")
    t.is_nil(core.claim_identity_from_values)
    t.is_nil(core.claim_identity)
  end,

  test_assigned_issue_count_counts_verified_query_rows = function()
    local calls = {}
    local github = {
      issue_list_open_assigned = function(repo, assignee, timeout)
        table.insert(calls, { repo = repo, assignee = assignee, timeout = timeout })
        return {
          stdout = '[{"number":42,"title":"Work","assignees":[{"login":"fkst-test-bot"}]}]',
          stderr = "",
          exit_code = 0,
        }
      end,
    }

    local verdict = core.self_assigned_open_issue_verdict(github, identity())

    t.eq(verdict.ok, true)
    t.eq(verdict.count, 1)
    t.eq(verdict.source_ref.kind, "github-assignee-query")
    t.eq(calls[1].repo, "owner/repo")
    t.eq(calls[1].assignee, "fkst-test-bot")
  end,

  test_assigned_issue_count_fails_closed_on_query_failure = function()
    local github = {
      issue_list_open_assigned = function()
        error("synthetic gh failure", 0)
      end,
    }

    local verdict = core.self_assigned_open_issue_verdict(github, identity())

    t.eq(verdict.ok, false)
    t.eq(verdict.error_class, "idle-assignee-query-failed")
    t.is_true(verdict.why:find("self-assigned issue query failed", 1, true) ~= nil)
  end,

  test_assigned_issue_count_fails_closed_on_malformed_query_json = function()
    local github = {
      issue_list_open_assigned = function()
        return { stdout = "not json", stderr = "", exit_code = 0 }
      end,
    }

    local verdict = core.self_assigned_open_issue_verdict(github, identity())

    t.eq(verdict.ok, false)
    t.eq(verdict.error_class, "idle-assignee-query-malformed")
    t.is_true(verdict.why:find("malformed self-assigned issue query", 1, true) ~= nil)
  end,

  test_assigned_issue_count_fails_closed_on_missing_query_stdout = function()
    local github = {
      issue_list_open_assigned = function()
        return { stderr = "", exit_code = 0 }
      end,
    }

    local verdict = core.self_assigned_open_issue_verdict(github, identity())

    t.eq(verdict.ok, false)
    t.eq(verdict.error_class, "idle-assignee-query-malformed")
    t.is_true(verdict.why:find("missing self-assigned issue query stdout", 1, true) ~= nil)
  end,

  test_system_idle_payload_is_small_and_source_ref_backed = function()
    local payload = core.build_system_idle_payload("2026-06-19T01:00:00Z", identity().source_ref, "2026-06-19T01:10:00Z")

    t.eq(payload.schema, "idle-detector.system-idle.v1")
    t.eq(payload.detected_at, "2026-06-19T01:00:00Z")
    t.eq(payload.source_ref.kind, "github-assignee-query")
    t.eq(payload.source_ref.ref, "owner/repo#issues?state=open&assignee=fkst-test-bot")
    t.eq(payload.expires_at, "2026-06-19T01:10:00Z")
    t.is_nil(payload.queues)
    t.is_nil(payload.metrics)
  end,

  test_freshness_verdict_is_pure_and_deterministic = function()
    local reference = core.iso_timestamp_epoch_seconds("2026-06-19T01:00:00Z")
    t.eq(core.freshness_verdict(reference, reference + 60, 600), "fresh")
    t.eq(core.freshness_verdict(reference, reference + 600, 600), "fresh")
    t.eq(core.freshness_verdict(reference, reference + 601, 600), "stale")
    t.eq(core.freshness_verdict(reference, reference - 60, 600), "fresh")
    t.raises(function() core.freshness_verdict(nil, reference, 600) end)
  end,

  test_iso_timestamp_parser_covers_invalid_and_january_dates = function()
    t.eq(core.iso_timestamp_epoch_seconds("not-a-time"), nil)
    t.eq(core.iso_timestamp_epoch_seconds("2026-13-01T00:00:00Z"), nil)
    t.eq(core.iso_timestamp_epoch_seconds("2026-01-01T00:00:00Z"), 1767225600)
  end,

  test_skip_fact_fields_are_pure_and_structured = function()
    for _, case in ipairs({
      { why = "busy self_assigned_open_issues=1" },
      { why = "missing FKST_GITHUB_BOT_LOGIN" },
      { why = "self-assigned issue query failed: gh unavailable" },
      { why = "malformed or missing idle_tick slot" },
      { why = "stale idle_tick slot" },
    }) do
      local fact = core.skip_fact("idle_gate", {
        queue = "idle_tick",
        payload = {
          source_ref = { kind = "cron", ref = "idle-detector/idle_poll/2099-01-01T00:00:00Z" },
        },
      }, case.why, true)
      t.is_true(fact:find("tag=SKIP", 1, true) ~= nil)
      t.is_true(fact:find("error_class=terminal-skip", 1, true) ~= nil)
      t.is_true(fact:find("source_ref=cron:idle-detector/idle_poll/2099-01-01T00:00:00Z", 1, true) ~= nil)
      t.is_true(fact:find("terminal=true", 1, true) ~= nil)
      t.is_true(fact:find("WHY=" .. case.why, 1, true) ~= nil)
    end
  end,
}
