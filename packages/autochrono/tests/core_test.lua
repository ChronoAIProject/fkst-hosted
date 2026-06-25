local core = require("core")
local strings = require("contract.strings")
local t = fkst.test

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
      extra = "ignored",
    },
    dedup_key = "owner/repo#issue#42@2026-06-03T01:02:03Z",
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function without(field)
  local value = issue()
  value[field] = nil
  return value
end

local function merge(base, extra)
  local value = {}
  for key, field in pairs(base) do
    value[key] = field
  end
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

return {
  test_proposal_id_round_trips_repo_and_issue = function()
    local id = core.proposal_id("owner/repo", 42)
    local repo, issue_number = core.parse_proposal_id(id)

    t.eq(id, "autochrono/issue/owner/repo/42")
    t.eq(repo, "owner/repo")
    t.eq(issue_number, "42")
  end,

  test_issue_ref_round_trips_rejects_non_canonical_inputs = function()
    t.eq(core.issue_ref_round_trips("owner:repo", 42), false)
    t.eq(core.issue_ref_round_trips(string.rep("r", 101), 42), false)
    t.eq(core.issue_ref_round_trips("owner/repo", "42:evil"), false)
    t.eq(core.issue_ref_round_trips("owner/repo", string.rep("7", 31)), false)
  end,

  test_parse_proposal_id_rejects_foreign_ids = function()
    local repo, issue_number = core.parse_proposal_id("consensus/issue/owner/repo/42")
    t.is_nil(repo)
    t.is_nil(issue_number)

    repo, issue_number = core.parse_proposal_id("autochrono/pull/owner/repo/42")
    t.is_nil(repo)
    t.is_nil(issue_number)

    repo, issue_number = core.parse_proposal_id("autochrono/issue/42")
    t.is_nil(repo)
    t.is_nil(issue_number)
  end,

  test_proposal_cache_key_is_versioned_and_path_safe = function()
    local key = core.proposal_cache_key("owner/repo", 42, "2026-06-03T01:02:03Z")

    t.eq(key, "autochrono/proposed/v1/owner/repo/issue/42/updated/2026-06-03T01-02-03Z")
    t.eq(key:find(":", 1, true), nil)
    t.eq(key:find("@", 1, true), nil)
    t.eq(key:find(" "), nil)
  end,

  test_sanitize_key_replaces_unsafe_characters = function()
    t.eq(strings.sanitize_key("2026-06-03T01:02:03Z", 200), "2026-06-03T01-02-03Z")
    t.eq(strings.sanitize_key("owner repo@example:42", 200), "owner-repo-example-42")
    t.eq(strings.sanitize_key("../repo", 200), "-/repo")
    t.eq(strings.sanitize_key("", 200), "empty")
  end,

  test_reply_dedup_key_is_stable_across_updates = function()
    local first = core.reply_dedup_key("owner/repo", 42)
    local second = core.reply_dedup_key("owner/repo", 42)
    local after_update = core.reply_dedup_key(issue().repo, issue({ updated_at = "2026-06-04T05:06:07Z" }).issue_number)

    t.eq(first, "autochrono:owner/repo#issue/42")
    t.eq(first, second)
    t.eq(first, after_update)
    t.eq(first:find("2026", 1, true), nil)
  end,

  test_normalize_source_ref_drops_extra_fields = function()
    local normalized = core.normalize_source_ref(issue().source_ref)

    t.eq(normalized.kind, "external")
    t.eq(normalized.ref, "owner/repo#issue/42")
    t.is_nil(normalized.extra)
  end,

  test_content_fetch_manifest_is_derived_from_source_ref = function()
    local manifest = core.content_fetch_manifest(issue().source_ref)

    t.is_true(#manifest <= 4000)
    t.is_true(manifest:find("source_ref owner/repo#issue/42", 1, true) ~= nil)
    t.is_true(manifest:find("full issue body", 1, true) ~= nil)
    t.is_true(manifest:find("ALL comments", 1, true) ~= nil)
    t.is_true(manifest:find("Body above is only a brief", 1, true) ~= nil)
  end,

  test_is_eligible_accepts_open_complete_issue = function()
    t.eq(core.is_eligible(issue()), true)
  end,

  test_is_eligible_rejects_non_round_tripping_issue_ref = function()
    t.eq(core.is_eligible(issue({ repo = "owner repo" })), false)
    t.eq(core.is_eligible(issue({ repo = "owner:repo" })), false)
    t.eq(core.is_eligible(issue({ repo = string.rep("r", 101) })), false)
    t.eq(core.is_eligible(issue({ issue_number = "42:evil" })), false)
    t.eq(core.is_eligible(issue({ issue_number = string.rep("7", 31) })), false)
  end,

  test_is_eligible_rejects_incomplete_or_closed_issue = function()
    t.eq(core.is_eligible(issue({ state = "CLOSED" })), false)
    t.eq(core.is_eligible(issue({ schema = "other.issue.v1" })), false)
    t.eq(core.is_eligible(without("repo")), false)
    t.eq(core.is_eligible(without("issue_number")), false)
    t.eq(core.is_eligible(without("title")), false)
    t.eq(core.is_eligible(without("url")), false)
    t.eq(core.is_eligible(without("updated_at")), false)
    t.eq(core.is_eligible(without("source_ref")), false)
    t.eq(core.is_eligible({}), false)
    t.eq(core.is_eligible(nil), false)
  end,

  test_proposal_dedup_key_stays_bounded = function()
    -- a pathological updated_at must not push dedup_key past the consensus 200-char cap
    local key = core.proposal_dedup_key("owner/repo", 42, string.rep("x", 500))
    t.is_true(#key <= 200)
    t.eq(key:find("autochrono/issue/owner/repo/42/", 1, true), 1)
  end,

  test_validate_proposal_accepts_well_formed = function()
    local mapping = require("departments.propose.mapping")
    t.eq(core.validate_proposal(mapping.build_proposal(issue())), true)
  end,

  test_render_template_missing_var_fails_closed = function()
    local ok = pcall(core.render_template, "Issue {{issue_number}} in {{repo}}", { repo = "owner/repo" })
    local exact_ok = pcall(core.render_template, "{{missing}}", {})

    t.eq(ok, false)
    t.eq(exact_ok, false)
  end,

  test_render_template_is_single_pass = function()
    t.eq(core.render_template("{{a}}", { a = "{{b}}", b = "ignored" }), "{{b}}")
  end,

  test_render_template_ignores_extra_vars = function()
    t.eq(core.render_template("{{a}}", { a = "x", unused = "y" }), "x")
  end,

  test_build_proposal_body_renders_issue_fields = function()
    local mapping = require("departments.propose.mapping")
    local payload = mapping.build_proposal(issue())

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
  end,

  test_build_proposal_throws_for_oversized_issue_title = function()
    local mapping = require("departments.propose.mapping")
    local ok = pcall(mapping.build_proposal, issue({ title = string.rep("x", 241) }))

    t.eq(ok, false)
  end,

  test_validate_proposal_rejects_contract_violations = function()
    local mapping = require("departments.propose.mapping")
    local ok = mapping.build_proposal(issue())
    t.eq(core.validate_proposal(merge(ok, { dedup_key = string.rep("a", 201) })), false)  -- over 200 cap
    t.eq(core.validate_proposal(merge(ok, { proposal_id = "autochrono/issue/owner/repo:42" })), false)  -- not path-safe
    t.eq(core.validate_proposal(merge(ok, { proposal_id = "other/issue/owner/repo/42" })), false)  -- not parseable
    t.eq(core.validate_proposal(merge(ok, { proposal_id = "autochrono/issue/owner/repo//42" })), false)  -- non-canonical
    t.eq(core.validate_proposal(merge(ok, { body = "" })), false)  -- empty body
    t.eq(core.validate_proposal(merge(ok, { source_ref = { kind = "external" } })), false)  -- ref missing
    t.eq(core.validate_proposal(merge(ok, { content_fetch = "" })), false)
    t.eq(core.validate_proposal(merge(ok, { content_fetch = string.rep("a", 4001) })), false)
    t.eq(core.validate_proposal(merge(ok, { schema = "other" })), false)
  end,

  test_validate_reached_accepts_and_rejects = function()
    local ok = {
      proposal_id = "autochrono/issue/owner/repo/42",
      body = "A concrete reply.",
      source_ref = { kind = "external", ref = "owner/repo#issue/42" },
    }
    t.eq(core.validate_reached(ok), true)
    t.eq(core.validate_reached(merge(ok, { body = "" })), false)
    t.eq(core.validate_reached(merge(ok, { body = string.rep("y", 12001) })), false)
    t.eq(core.validate_reached(merge(ok, { proposal_id = "" })), false)
    t.eq(core.validate_reached({ proposal_id = "autochrono/issue/owner/repo/42", body = "x" }), false)  -- no source_ref
  end,
}
