local t = fkst.test
local core = require("core")

local fixed_now_seconds = 1780459323

local function with_env(value_by_name, fn)
  local old_read_env = core.read_env
  core.read_env = function(name)
    return value_by_name[name] or ""
  end
  local ok, result = pcall(fn)
  core.read_env = old_read_env
  if not ok then
    error(result, 0)
  end
  return result
end

local function pr(fields)
  fields = fields or {}
  return {
    number = fields.number or 7,
    state = fields.state or "OPEN",
    author_login = fields.author_login or "contributor",
    head_ref_name = fields.head_ref_name or "feature/contrib",
    created_at = fields.created_at,
    updated_at = fields.updated_at,
  }
end

local managed = {
  ["fkst-test-bot"] = true,
  ["other-bot"] = true,
}

local function merged_marker(issue_number, pr_number)
  return '<!-- fkst:github-devloop:merged:v1 proposal="github-devloop/issue/owner/repo/'
    .. tostring(issue_number)
    .. '" pr="'
    .. tostring(pr_number)
    .. '" version="v1" head_sha="0123456789abcdef0123456789abcdef01234567" -->'
end

local function merged_state_marker(issue_number)
  return '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/repo/'
    .. tostring(issue_number)
    .. '" state="merged" version="v1" stage_rank="900" -->'
end

local function is_candidate_with_default_age(candidate)
  return with_env({ FKST_EXTERNAL_PR_BRIDGE_MIN_AGE_SECONDS = "" }, function()
    return core.is_external_candidate(candidate, managed, fixed_now_seconds)
  end)
end

return {
  test_iso_timestamp_epoch_seconds_parses_utc_timestamps = function()
    t.eq(core.iso_timestamp_epoch_seconds("1970-01-01T00:00:00Z"), 0)
    t.eq(core.iso_timestamp_epoch_seconds("1970-01-02T00:00:00Z"), 86400)
    t.eq(core.iso_timestamp_epoch_seconds("2000-02-29T12:34:56Z"), 951827696)
    t.eq(core.iso_timestamp_epoch_seconds("2026-06-03T01:02:03Z"), 1780448523)
    t.eq(core.iso_timestamp_epoch_seconds("2026-06-03T01:02:03.123Z"), 1780448523)
  end,

  test_iso_timestamp_epoch_seconds_rejects_invalid_timestamps = function()
    t.is_nil(core.iso_timestamp_epoch_seconds(nil))
    t.is_nil(core.iso_timestamp_epoch_seconds(""))
    t.is_nil(core.iso_timestamp_epoch_seconds("2026-06-03T01:02:03+00:00"))
    t.is_nil(core.iso_timestamp_epoch_seconds("2026-02-29T01:02:03Z"))
    t.is_nil(core.iso_timestamp_epoch_seconds("2026-06-03T24:02:03Z"))
  end,

  test_external_pr_bridge_min_age_seconds_reads_positive_integer_parameter = function()
    with_env({ FKST_EXTERNAL_PR_BRIDGE_MIN_AGE_SECONDS = "42" }, function()
      t.eq(core.external_pr_bridge_min_age_seconds(), 42)
    end)
  end,

  test_external_pr_bridge_min_age_seconds_falls_back_to_default = function()
    local default = core.DEFAULT_EXTERNAL_PR_BRIDGE_MIN_AGE_SECONDS
    for _, value in ipairs({ "", "abc", "0", "-1", "3.0" }) do
      with_env({ FKST_EXTERNAL_PR_BRIDGE_MIN_AGE_SECONDS = value }, function()
        t.eq(core.external_pr_bridge_min_age_seconds(), default)
      end)
    end
  end,

  test_read_env_command_allows_external_pr_min_age_parameter = function()
    t.eq(
      core.read_env_command("FKST_EXTERNAL_PR_BRIDGE_MIN_AGE_SECONDS"),
      'printf %s "$FKST_EXTERNAL_PR_BRIDGE_MIN_AGE_SECONDS"'
    )
  end,

  test_normalize_pr_exposes_created_at_from_gh_view_and_rest_shapes = function()
    t.eq(core.normalize_pr({ number = 7, createdAt = "2026-06-03T01:02:03Z" }, "owner/repo").created_at, "2026-06-03T01:02:03Z")
    t.eq(core.normalize_pr({ number = 8, created_at = "2026-06-03T02:02:03Z" }, "owner/repo").created_at, "2026-06-03T02:02:03Z")
  end,

  test_external_candidate_skips_young_pr = function()
    local candidate = pr({ created_at = "2026-06-03T04:01:03Z" })
    t.eq(is_candidate_with_default_age(candidate), false)
  end,

  test_external_candidate_accepts_exact_threshold_age = function()
    local candidate = pr({ created_at = "2026-06-03T01:02:03Z" })
    t.eq(is_candidate_with_default_age(candidate), true)
  end,

  test_external_candidate_accepts_old_pr = function()
    local candidate = pr({ created_at = "2026-06-03T01:02:02Z" })
    t.eq(is_candidate_with_default_age(candidate), true)
  end,

  test_external_candidate_uses_created_at_not_recent_updated_at = function()
    local candidate = pr({
      created_at = "2026-06-03T01:02:02Z",
      updated_at = "2026-06-03T04:01:59Z",
    })
    t.eq(is_candidate_with_default_age(candidate), true)
  end,

  test_external_candidate_skips_missing_or_unparseable_created_at = function()
    t.eq(is_candidate_with_default_age(pr({ created_at = nil })), false)
    t.eq(is_candidate_with_default_age(pr({ created_at = "not-a-time" })), false)
  end,

  test_find_bridge_issue_merged_signal_ignores_untrusted_merged_marker = function()
    local issue = {
      comments = {
        { author_login = "contributor", body = merged_marker(77, 88) },
      },
    }

    t.is_nil(core.find_bridge_issue_merged_signal(issue, "owner/repo", 77, managed))
  end,

  test_find_bridge_issue_merged_signal_ignores_untrusted_state_marker = function()
    local issue = {
      comments = {
        { author_login = "contributor", body = merged_state_marker(77) },
      },
    }

    t.is_nil(core.find_bridge_issue_merged_signal(issue, "owner/repo", 77, managed))
  end,

  test_find_pr_bridge_marker_ignores_untrusted_bridge_marker = function()
    local comments = {
      { author_login = "contributor", body = core.bridge_marker("owner/repo", 7, 77) },
    }

    t.is_nil(core.find_pr_bridge_marker(comments, "owner/repo", 7, managed))
  end,

  test_find_pr_handled_marker_ignores_untrusted_handled_marker = function()
    local comments = {
      { author_login = "contributor", body = core.handled_marker("owner/repo", 7, 77) },
    }

    t.is_nil(core.find_pr_handled_marker(comments, "owner/repo", 7, 77, managed))
  end,
}
