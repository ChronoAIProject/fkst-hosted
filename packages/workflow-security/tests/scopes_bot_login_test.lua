-- Regression: github-issue.scopes.trusted_text must count a comment as bot-authored
-- even when `gh issue view --json comments` returns the author login WITHOUT the
-- "[bot]" suffix (GraphQL) while FKST_GITHUB_BOT_LOGIN carries it (REST form). Before
-- the normalize_login fix the two never matched, so the bot's own blueprint marker was
-- excluded from the trusted text, latest_blueprint returned nil, and the reconcile
-- stalled forever at "no trusted blueprint marker" (workflow-security/writer never ran).
local scopes = require("github-issue.scopes")
local t = fkst.test

local function issue_with_bot_comment(author_login)
  return {
    body = "issue body",
    comments = {
      { author = { login = author_login }, body = "<!-- fkst:blueprint:v1 -->" },
      { author = { login = "some-human" }, body = "unrelated human comment" },
    },
  }
end

return {
  -- The core regression: comment author "slug" (GraphQL) vs bot_login "slug[bot]" (REST).
  test_trusted_text_matches_bot_across_graphql_and_rest_login_forms = function()
    local text = scopes.trusted_text(
      issue_with_bot_comment("chronoai-fkst-test"),
      "chronoai-fkst-test[bot]"
    )
    t.is_true(text:find("fkst:blueprint:v1", 1, true) ~= nil)
    t.is_true(text:find("unrelated human comment", 1, true) == nil)
  end,

  -- Symmetric case: comment carries the [bot] suffix, configured bot_login is bare.
  test_trusted_text_matches_when_comment_login_has_bot_suffix = function()
    local text = scopes.trusted_text(
      issue_with_bot_comment("chronoai-fkst-test[bot]"),
      "chronoai-fkst-test"
    )
    t.is_true(text:find("fkst:blueprint:v1", 1, true) ~= nil)
  end,

  -- App-actor form: gh returns "app/<slug>" for an App author on some surfaces
  -- (e.g. `issue view --json author`); it must still match the pinned bot.
  test_trusted_text_matches_when_login_has_app_prefix = function()
    local text = scopes.trusted_text(
      issue_with_bot_comment("app/chronoai-fkst-test"),
      "chronoai-fkst-test[bot]"
    )
    t.is_true(text:find("fkst:blueprint:v1", 1, true) ~= nil)
  end,

  -- A non-bot author is still excluded (trust boundary preserved).
  test_trusted_text_excludes_non_bot_author = function()
    local text = scopes.trusted_text(
      issue_with_bot_comment("attacker"),
      "chronoai-fkst-test[bot]"
    )
    t.is_true(text:find("fkst:blueprint:v1", 1, true) == nil)
  end,

  -- No bot pinned -> trust all comments (unchanged behavior).
  test_trusted_text_trusts_all_when_bot_login_empty = function()
    local text = scopes.trusted_text(issue_with_bot_comment("anyone"), "")
    t.is_true(text:find("fkst:blueprint:v1", 1, true) ~= nil)
    t.is_true(text:find("unrelated human comment", 1, true) ~= nil)
  end,
}
