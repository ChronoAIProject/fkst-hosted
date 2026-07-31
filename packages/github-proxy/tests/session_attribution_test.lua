-- Attribution must survive every partial-identity case: a comment is never
-- worth losing over a footer.
local h = require("tests.proxy_integration_helpers")
local t = h.t
local session_attribution = require("github-proxy-effects.core.session_attribution")

local function reader(values)
  return function(name)
    return values[name]
  end
end

local function build(values, repo)
  return session_attribution.build(reader(values), repo)
end

local FULL = {
  FKST_SESSION_ID = "6859865f-a61c-5145-a76f-f11771f31e89",
  FKST_WORK_LABEL_NAMESPACE = "chronoai-fkst-cloud",
  FKST_TRIGGER_ISSUE = "5730",
}

return {
  test_full_identity_links_to_the_trigger_issue = function()
    t.eq(
      build(FULL, "ChronoAIProject/fkst-hosted"),
      "Written by session: [chronoai-fkst-cloud-6859865f-a61c-5145-a76f-f11771f31e89]"
        .. "(https://github.com/ChronoAIProject/fkst-hosted/issues/5730)"
    )
  end,

  -- Two sessions on one issue must be tellable apart -- the whole point.
  test_two_sessions_produce_different_attributions = function()
    local a = build(FULL, "owner/repo")
    local b = build({
      FKST_SESSION_ID = "433ac5ff-ea3a-5bf7-a812-16dee7781989",
      FKST_WORK_LABEL_NAMESPACE = "chronoai-fkst-cloud",
      FKST_TRIGGER_ISSUE = "5410",
    }, "owner/repo")
    t.is_true(a ~= b)
    t.is_true(b:find("5410", 1, true) ~= nil)
  end,

  test_missing_trigger_issue_drops_the_link_not_the_footer = function()
    t.eq(
      build({ FKST_SESSION_ID = "sid", FKST_WORK_LABEL_NAMESPACE = "ns" }, "owner/repo"),
      "Written by session: ns-sid"
    )
  end,

  test_missing_repo_drops_the_link_not_the_footer = function()
    t.eq(
      build({ FKST_SESSION_ID = "sid", FKST_WORK_LABEL_NAMESPACE = "ns", FKST_TRIGGER_ISSUE = "7" }, nil),
      "Written by session: ns-sid"
    )
  end,

  test_missing_namespace_uses_the_session_id_alone = function()
    t.eq(
      build({ FKST_SESSION_ID = "sid", FKST_TRIGGER_ISSUE = "7" }, "owner/repo"),
      "Written by session: [sid](https://github.com/owner/repo/issues/7)"
    )
  end,

  -- A standalone package deployment has no session to attribute.
  test_no_session_id_yields_no_footer = function()
    t.eq(build({ FKST_WORK_LABEL_NAMESPACE = "ns", FKST_TRIGGER_ISSUE = "7" }, "owner/repo"), nil)
    t.eq(build({ FKST_SESSION_ID = "   " }, "owner/repo"), nil)
  end,

  -- A throwing reader is a failed subprocess, not a reason to drop the comment.
  test_throwing_reader_degrades_instead_of_erroring = function()
      local partial = function(name)
      if name == "FKST_SESSION_ID" then
        return "sid"
      end
      error("env read failed")
    end
    t.eq(session_attribution.build(partial, "owner/repo"), "Written by session: sid")
  end,

  test_absent_reader_yields_no_footer = function()
      t.eq(session_attribution.build(nil, "owner/repo"), nil)
  end,

  -- parse_command reads the first non-empty line; the footer must never look
  -- like a command, wherever it lands.
  test_footer_is_not_parseable_as_an_operator_command = function()
    local footer = build(FULL, "owner/repo")
    t.eq(footer:match("^fkst:%s*([%w_-]+)"), nil)
  end,
}
