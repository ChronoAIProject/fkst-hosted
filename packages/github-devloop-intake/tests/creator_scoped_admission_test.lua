local entity_lib = require("devloop.entity")
local h = require("tests.devloop_helpers")
local t = h.t
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

local function terminal_observe(number)
  return {
    schema_version = 1,
    generated_at_ms = 1784768523000,
    source = {
      durable_root = "/tmp/fkst-durable",
      database = "/tmp/fkst-durable/delivery.redb",
      read_semantics = "single read transaction",
      history_semantics = "delivery queue snapshot only",
    },
    limits = { max_deliveries = 10000, max_dead_letters = 10000 },
    truncated = { deliveries = false, dead_letters = false },
    queues = json.decode("[]"),
    deliveries = json.decode("[]"),
    dead_letters = {
      {
        delivery_id = "creator-scoped-terminal-" .. tostring(number),
        queue = "github-devloop-intake.devloop_intake_candidate",
        dept = "github-devloop-intake-default.intake_judge",
        source = { kind = "external", reference = source_ref(number).ref },
        attempts = 1,
        permanent = true,
        replayable = false,
        dead_at_ms = 1784768523000,
      },
    },
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

  test_not_routed_observed_issue_stops_after_metadata_without_content = function()
    mock_creator_env("creator-login")
    t.mock_observe(terminal_observe(48))
    mock_metadata(48, { assignees = { "other-login" } })

    local result = h.run_department(
      "departments/admission/main.lua",
      observed_event(48),
      opts("creator-observed-skip-not-routed", "creator-login")
    )

    t.eq(result.exit_code, 0)
    t.eq(candidate(result), nil)
    t.eq(count_calls("--json " .. metadata_fields), 1)
    t.eq(count_calls("--json " .. content_fields), 0)
    t.eq(count_calls("--add-label"), 0)
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
