local entity_lib = require("devloop.entity")
local devloop_base = require("devloop.base")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local gh_argv = require("testkit.gh_argv_mock")

local metadata_fields = "number,state,labels,assignees,author"
local content_fields = "title,body,createdAt,updatedAt,labels,comments,state,assignees,author"
local claimed_label = "fkst-dev:claimed"

local function source_ref(number)
  return entity_lib.issue_source_ref("owner/repo", number)
end

local function event(number)
  return {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "issue",
      repo = "owner/repo",
      number = number,
      title = "Untrusted event title",
      state = "OPEN",
      labels = {},
      updated_at = "2026-07-23T01:02:03Z",
      dedup_key = "owner/repo#issue#" .. tostring(number) .. "@2026-07-23T01:02:03Z",
      source_ref = source_ref(number),
    },
    source_ref = source_ref(number),
  }
end

local function observed_event(number)
  return {
    queue = "github-proxy.github_issue_observed",
    payload = {
      schema = "github-proxy.issue-observed.v1",
      type = "issue",
      repo = "owner/repo",
      number = number,
      updated_at = "2026-07-23T01:02:03Z",
      dedup_key = "github-issue-observed/owner/repo/" .. tostring(number) .. "/2026-07-23T01:02:03Z/poll",
      source_ref = source_ref(number),
    },
    source_ref = source_ref(number),
  }
end

local function opts(name, creator, authorized_logins)
  return h.opts(name, {
    FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
    FKST_GITHUB_CLAIM_MODE = "label",
    FKST_GITHUB_WRITE = "1",
    FKST_GITHUB_REPO = "owner/repo",
    FKST_SESSION_CREATOR = creator or "",
    FKST_SESSION_WORK_LABEL = "shared-work",
    FKST_DEVLOOP_MANAGED_BOT_LOGINS = "fkst-test-bot",
    FKST_GITHUB_AUTHORIZED_LOGINS = authorized_logins == nil and "trusted-human" or authorized_logins,
  })
end

local function mock_creator_env(creator)
  for _ = 1, 16 do
    t.mock_command('printf %s "$FKST_SESSION_CREATOR"', {
      stdout = creator or "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_GITHUB_CLAIM_MODE"', {
      stdout = "label",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL"', {
      stdout = "shared-work",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
      stdout = "1",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function issue_fields(number, fields)
  local selected = fields or {}
  return {
    number = number,
    title = selected.title or "Authorized work",
    body = selected.body or "Authorized body",
    updated_at = "2026-07-23T01:02:03Z",
    state = selected.state or "OPEN",
    labels = selected.labels or { "shared-work" },
    comments = selected.comments or {},
    assignees = selected.assignees or { "creator-login" },
    author_login = selected.author_login or "trusted-human",
  }
end

local function mock_metadata(number, fields)
  t.mock_command("gh issue view " .. tostring(number) .. " --repo owner/repo --json " .. metadata_fields, {
    stdout = entity_read_mocks.issue_view_stdout(issue_fields(number, fields)),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_content(number, fields)
  entity_read_mocks.mock_issue_view_selector(t, issue_fields(number, fields), content_fields)
end

local function mock_legacy_claim_view(number, fields)
  entity_read_mocks.mock_issue_view_selector(t, issue_fields(number, fields), "assignees,author,labels")
end

local function mock_add_claim(number)
  t.mock_command("gh issue edit " .. tostring(number) .. " --repo owner/repo --add-label '" .. claimed_label .. "'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_remove_claim(number)
  t.mock_command("gh issue edit " .. tostring(number) .. " --repo owner/repo --remove-label '" .. claimed_label .. "'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function candidate(result)
  return h.find_raise(result.raises, "devloop_intake_candidate")
end

local function trusted_reintake_command()
  return {
    id = "IC_creator_reintake",
    body = "fkst: reintake",
    author_login = devloop_base.trusted_bot_login(),
    created_at = "2026-07-23T01:03:00Z",
  }
end

local function count_calls(needle)
  local count = 0
  for _, call in ipairs(t.command_calls()) do
    if gh_argv.call_contains(call, needle) then
      count = count + 1
    end
  end
  return count
end

local function first_call_index(needle)
  for index, call in ipairs(t.command_calls()) do
    if gh_argv.call_contains(call, needle) then
      return index
    end
  end
  return nil
end

local function run(number, run_opts)
  return h.run_department("departments/admission/main.lua", event(number), run_opts)
end

return {
  test_not_routed_issue_stops_after_metadata_without_content_or_claim = function()
    mock_creator_env("creator-login")
    mock_metadata(42, { assignees = { "other-login" } })

    local result = run(42, opts("creator-skip-not-routed", "creator-login"))

    t.eq(result.exit_code, 0)
    t.eq(candidate(result), nil)
    t.eq(count_calls("--json " .. metadata_fields), 1)
    t.eq(count_calls("--json " .. content_fields), 0)
    t.eq(count_calls("--add-label"), 0)
    t.eq(#result.raises, 0)
  end,

  test_label_mode_observed_issue_skips_before_observe_or_github_read = function()
    mock_creator_env("creator-login")

    local result = h.run_department(
      "departments/admission/main.lua",
      observed_event(48),
      opts("creator-observed-skip-label-mode", "creator-login")
    )

    t.eq(result.exit_code, 0)
    t.eq(candidate(result), nil)
    t.eq(count_calls("--json " .. metadata_fields), 0)
    t.eq(count_calls("--json " .. content_fields), 0)
    t.eq(count_calls("--add-label"), 0)
  end,

  test_creator_scoped_reintake_uses_marker_authority_despite_stale_active_label = function()
    local proposal_id = "github-devloop/issue/owner/repo/49"
    local labels = { "shared-work", claimed_label, "fkst-dev:enabled", "fkst-dev:thinking" }
    local comments = {
      core.state_marker(proposal_id, "thinking", proposal_id .. "/2026-07-23T01-00-00Z"),
      core.state_marker(proposal_id, "blocked", proposal_id .. "/2026-07-23T01-00-00Z/timeout-reconcile/thinking/4"),
      trusted_reintake_command(),
    }
    mock_creator_env("creator-login")
    mock_metadata(49, { labels = labels })
    mock_content(49, { labels = labels, comments = comments })
    mock_metadata(49, { labels = labels })

    local result = run(49, opts("creator-marker-authoritative-reintake", "creator-login"))

    t.eq(result.exit_code, 0)
    t.is_true(candidate(result) ~= nil)
    t.eq(candidate(result).payload.issue_number, "49")
    t.eq(candidate(result).payload.reintake_command_created_at, "2026-07-23T01:03:00Z")
    t.eq(count_calls("--json " .. metadata_fields), 2)
    t.eq(count_calls("--json " .. content_fields), 1)
    t.eq(count_calls("--add-label"), 0)
  end,

  test_creator_scoped_blocked_issue_without_command_does_not_claim_or_raise = function()
    local labels = { "shared-work", claimed_label, "fkst-dev:enabled", "fkst-dev:blocked" }
    mock_creator_env("creator-login")
    mock_metadata(50, { labels = labels })
    mock_content(50, { labels = labels, comments = {} })

    local result = run(50, opts("creator-blocked-without-reintake", "creator-login"))

    t.eq(result.exit_code, 0)
    t.eq(candidate(result), nil)
    t.eq(count_calls("--json " .. metadata_fields), 1)
    t.eq(count_calls("--json " .. content_fields), 1)
    t.eq(count_calls("--add-label"), 0)
    t.eq(#result.raises, 0)
  end,

  test_out_of_scope_issue_stops_after_metadata_without_content_or_claim = function()
    mock_creator_env("creator-login")
    mock_metadata(43, { labels = { "other-work" } })

    local result = run(43, opts("creator-skip-work-scope", "creator-login"))

    t.eq(result.exit_code, 0)
    t.eq(candidate(result), nil)
    t.eq(count_calls("--json " .. metadata_fields), 1)
    t.eq(count_calls("--json " .. content_fields), 0)
    t.eq(count_calls("--add-label"), 0)
  end,

  test_untrusted_author_stops_before_claim_and_content_only_when_creator_is_set = function()
    mock_creator_env("creator-login")
    mock_metadata(44, { author_login = "untrusted-human" })

    local result = run(44, opts("creator-skip-untrusted-author", "creator-login", "trusted-human"))

    t.eq(result.exit_code, 0)
    t.eq(candidate(result), nil)
    t.eq(count_calls("--json " .. metadata_fields), 1)
    t.eq(count_calls("--json " .. content_fields), 0)
    t.eq(count_calls("--add-label"), 0)
    t.eq(#result.raises, 0)
  end,

  test_routed_authorized_issue_fetches_content_only_after_verified_claim = function()
    mock_creator_env("creator-login")
    mock_metadata(45, { assignees = { "CREATOR-LOGIN" } })
    mock_add_claim(45)
    mock_metadata(45, {
      labels = { "shared-work", claimed_label },
      assignees = { "creator-login" },
    })
    mock_content(45, {
      labels = { "shared-work", claimed_label },
      assignees = { "creator-login" },
    })

    local result = run(45, opts("creator-admit-after-claim", "creator-login"))

    t.eq(result.exit_code, 0)
    t.is_true(candidate(result) ~= nil)
    t.eq(candidate(result).payload.issue_number, "45")
    t.eq(count_calls("--json " .. metadata_fields), 2)
    t.eq(count_calls("--json " .. content_fields), 1)
    t.eq(count_calls("--add-label"), 1)
    local first_metadata = first_call_index("--json " .. metadata_fields)
    local claim = first_call_index("--add-label")
    local content = first_call_index("--json " .. content_fields)
    t.is_true(first_metadata < claim)
    t.is_true(claim < content)
  end,

  test_assignee_reroute_during_claim_verification_removes_label_without_content_fetch = function()
    mock_creator_env("creator-login")
    mock_metadata(46)
    mock_add_claim(46)
    mock_metadata(46, {
      labels = { "shared-work", claimed_label },
      assignees = { "other-login" },
    })
    mock_remove_claim(46)

    local result = run(46, opts("creator-claim-reroute-race", "creator-login"))

    t.eq(result.exit_code, 0)
    t.eq(candidate(result), nil)
    t.eq(count_calls("--add-label"), 1)
    t.eq(count_calls("--remove-label"), 1)
    t.eq(count_calls("--json " .. content_fields), 0)
  end,

  test_creator_unset_preserves_legacy_label_mode_author_and_full_view_flow = function()
    mock_creator_env("")
    mock_content(47, {
      assignees = {},
      author_login = "untrusted-human",
    })
    mock_add_claim(47)
    mock_legacy_claim_view(47, {
      labels = { "shared-work", claimed_label },
      assignees = {},
      author_login = "untrusted-human",
    })

    local result = run(47, opts("legacy-label-author-bypass", "", ""))

    t.eq(result.exit_code, 0)
    t.is_true(candidate(result) ~= nil)
    t.eq(count_calls("--json " .. metadata_fields), 0)
    t.eq(count_calls("--json " .. content_fields), 1)
    t.eq(count_calls("--add-label"), 1)
  end,
}
