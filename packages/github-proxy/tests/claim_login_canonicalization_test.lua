local core = require("github-proxy-effects.core")
local author_policy = require("testkit.github_author_policy")
local t = fkst.test

local repo = "owner/x"
local issue_number = 42

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
}
