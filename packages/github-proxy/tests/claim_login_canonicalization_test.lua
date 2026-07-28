local core = require("github-proxy-effects.core")
local author_policy = require("testkit.github_author_policy")
local t = fkst.test

local repo = "owner/x"
local issue_number = 42
local work_label_namespace = "chronoai-fkst-cloud-test"
local logical_work_label = "fkst-dev"

local function claim_payload(owner)
  return {
    claim = {
      owner = owner,
      source_ref = {
        kind = "external",
        ref = repo .. "#issue/" .. tostring(issue_number),
      },
    },
  }
end

local function mock_assignees(json)
  author_policy.mock_env(t)
  t.mock_command("gh api repos/owner/x/issues/42", {
    stdout = '{"assignees":' .. json .. "}\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_session_scope(value, reads)
  for _ = 1, reads or 1 do
    t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL"', {
      stdout = value or "",
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, (reads or 1) * 2 do
    t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL_MAP_JSON"', {
      stdout = string.format('{"%s":"%s"}', logical_work_label, value),
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, reads or 1 do
    t.mock_command('printf %s "$FKST_WORK_LABEL_NAMESPACE"', {
      stdout = work_label_namespace,
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_incomplete_session_scope(effective)
  t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  for _ = 1, 2 do
    t.mock_command('printf %s "$FKST_SESSION_WORK_LABEL_MAP_JSON"', {
      stdout = string.format('{"%s":"%s"}', logical_work_label, effective),
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command('printf %s "$FKST_WORK_LABEL_NAMESPACE"', {
    stdout = work_label_namespace,
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_live_claim_verification_canonicalizes_mixed_case_assignee = function()
    local payload = claim_payload("elonsg")
    mock_assignees('[{"login":"ElonSG"}]')

    t.is_true(core.verify_issue_claim_before_write(payload, repo, issue_number, "claim_test"))
  end,

  test_in_memory_claim_verification_canonicalizes_mixed_case_assignee = function()
    local payload = claim_payload("elonsg")
    local issue = { assignees = { { login = "ElonSG" } } }

    t.is_true(core.verify_issue_claim_in_issue(issue, payload, repo, issue_number, "claim_test"))
  end,

  test_live_claim_verification_canonicalizes_mixed_case_owner = function()
    local payload = claim_payload("ElonSG")
    mock_assignees('[{"login":"elonsg"}]')

    t.is_true(core.verify_issue_claim_before_write(payload, repo, issue_number, "claim_test"))
  end,

  test_in_memory_claim_verification_canonicalizes_mixed_case_owner = function()
    local payload = claim_payload("ElonSG")
    local issue = { assignees = { { login = "elonsg" } } }

    t.is_true(core.verify_issue_claim_in_issue(issue, payload, repo, issue_number, "claim_test"))
  end,

  test_live_claim_verification_canonicalizes_bot_suffix = function()
    local payload = claim_payload("elonsg")
    mock_assignees('[{"login":"ElonSG[bot]"}]')

    t.is_true(core.verify_issue_claim_before_write(payload, repo, issue_number, "claim_test"))
  end,

  test_in_memory_claim_verification_canonicalizes_bot_suffix = function()
    local payload = claim_payload("elonsg")
    local issue = { assignees = { { login = "ElonSG[bot]" } } }

    t.is_true(core.verify_issue_claim_in_issue(issue, payload, repo, issue_number, "claim_test"))
  end,

  test_claim_verification_refuses_different_assignee = function()
    local payload = claim_payload("elonsg")
    local issue = { assignees = { { login = "someone-else" } } }
    mock_assignees('[{"login":"someone-else"}]')

    t.eq(core.verify_issue_claim_before_write(payload, repo, issue_number, "claim_test"), false)
    t.eq(core.verify_issue_claim_in_issue(issue, payload, repo, issue_number, "claim_test"), false)
  end,

  test_claim_verification_refuses_empty_assignees = function()
    local payload = claim_payload("elonsg")
    local issue = { assignees = {} }
    mock_assignees("[]")

    t.eq(core.verify_issue_claim_before_write(payload, repo, issue_number, "claim_test"), false)
    t.eq(core.verify_issue_claim_in_issue(issue, payload, repo, issue_number, "claim_test"), false)
  end,

  test_claim_verification_refuses_multiple_assignees = function()
    local payload = claim_payload("elonsg")
    local issue = {
      assignees = {
        { login = "ElonSG" },
        { login = "someone-else" },
      },
    }
    mock_assignees('[{"login":"ElonSG"},{"login":"someone-else"}]')

    t.eq(core.verify_issue_claim_before_write(payload, repo, issue_number, "claim_test"), false)
    t.eq(core.verify_issue_claim_in_issue(issue, payload, repo, issue_number, "claim_test"), false)
  end,

  test_label_claim_requires_the_exact_namespaced_base_and_claim_labels = function()
    local base = "fkst-dev-chronoai-fkst-cloud-test"
    local payload = claim_payload("fkst-test-bot")
    payload.claim.mode = "label"
    payload.claim.label = base .. ":claimed"

    mock_session_scope(base)
    t.is_true(core.verify_issue_claim_in_issue({
      assignees = {},
      labels = { base, base .. ":claimed" },
    }, payload, repo, issue_number, "claim_test"))

    for _, labels in ipairs({
      { "fkst-dev", "fkst-dev:claimed" },
      { base .. ":claimed" },
      { base, "fkst-dev:claimed" },
      { "fkst-dev-another-provider", "fkst-dev-another-provider:claimed" },
    }) do
      mock_session_scope(base)
      t.eq(core.verify_issue_claim_in_issue({
        assignees = {},
        labels = labels,
      }, payload, repo, issue_number, "claim_test"), false)
    end
  end,

  test_namespaced_label_claim_fails_closed_when_session_work_labels_are_empty = function()
    local base = "fkst-dev-chronoai-fkst-cloud-test"
    local payload = claim_payload("fkst-test-bot")
    payload.claim.mode = "label"
    payload.claim.label = base .. ":claimed"

    mock_incomplete_session_scope(base)
    t.eq(core.verify_issue_claim_in_issue({
      assignees = {},
      labels = { base, base .. ":claimed" },
    }, payload, repo, issue_number, "claim_test"), false)
  end,
}
