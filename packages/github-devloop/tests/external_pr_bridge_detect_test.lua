-- Regression + contract-grammar tests for external-PR bridge marker detection.
--
-- #2342 review (blast-radius lens): bridge presence must be decided on the
-- COMPLETE `<!-- ...v1 ... -->` comment grammar, never on a bare-prefix
-- substring. A normal, trusted-bot-authored issue whose body merely MENTIONS
-- the marker prefix in prose is NOT a bridge issue: detect() must return nil
-- (normal implement path) and must NOT throw. The earlier substring has_marker
-- made detect() throw on such a body, breaking normal implement.
local t = fkst.test
local bridge = require("contract.external_pr_bridge")
local detect_bridge = require("departments.implement.external_pr_bridge")

local managed = { ["fkst-test-bot"] = true }

local function complete_marker_body()
  return table.concat({
    bridge.marker("owner/repo", 7),
    "",
    "- Source: external PR #7; contributor change already provisioned in your worktree.",
  }, "\n")
end

-- A normal feature issue that mentions the prefix in prose and a code fence,
-- but carries NO complete <!-- ... --> comment marker.
local function prose_prefix_body()
  return table.concat({
    "This issue discusses how the fkst:github-external-pr-intake:external-pr-bridge:v1",
    "marker format works. It is a normal feature issue, not an external PR bridge.",
    "```",
    'fkst:github-external-pr-intake:external-pr-bridge:v1 repo="x" pr="9"',
    "```",
  }, "\n")
end

local function assert_errors(fn)
  local ok = pcall(fn)
  t.eq(ok, false)
end

return {
  -- Grammar: presence is the complete-comment grammar, not a bare-prefix substring.
  test_has_marker_false_for_prose_prefix_without_complete_comment = function()
    t.eq(bridge.has_marker(prose_prefix_body()), false)
    t.is_nil(bridge.find_marker_comment(prose_prefix_body()))
    t.is_nil(bridge.find_marker(prose_prefix_body()))
  end,

  test_has_marker_true_for_complete_comment = function()
    t.eq(bridge.has_marker(complete_marker_body()), true)
    local marker = bridge.find_marker(complete_marker_body())
    t.eq(marker.repo, "owner/repo")
    t.eq(marker.pr_number, 7)
  end,

  -- A complete comment carrying the prefix but MISSING required attributes still
  -- fails closed (find_marker parses and errors), so a corrupt marker is never
  -- silently treated as absent.
  test_complete_comment_missing_attrs_fails_closed = function()
    local body = "<!-- fkst:github-external-pr-intake:external-pr-bridge:v1 -->"
    t.eq(bridge.has_marker(body), true)
    assert_errors(function()
      bridge.find_marker(body)
    end)
  end,

  -- REGRESSION: prose-prefix, trusted-bot body -> detect returns nil (normal
  -- path), NOT an error. Fails on the old substring has_marker.
  test_detect_returns_nil_for_prose_prefix_trusted_issue = function()
    local current = { body = prose_prefix_body(), author_login = "fkst-test-bot" }
    t.is_nil(detect_bridge.detect(current, "owner/repo", managed))
  end,

  test_detect_returns_marker_for_complete_trusted_marker = function()
    local current = { body = complete_marker_body(), author_login = "fkst-test-bot" }
    local marker = detect_bridge.detect(current, "owner/repo", managed)
    t.eq(marker.repo, "owner/repo")
    t.eq(marker.pr_number, 7)
  end,

  -- Fail-closed preserved: a complete marker in an untrusted body errors.
  test_detect_errors_for_complete_marker_untrusted_author = function()
    local current = { body = complete_marker_body(), author_login = "contributor" }
    assert_errors(function()
      detect_bridge.detect(current, "owner/repo", managed)
    end)
  end,

  -- Fail-closed preserved: a complete marker whose repo mismatches errors.
  test_detect_errors_for_repo_mismatch = function()
    local current = { body = complete_marker_body(), author_login = "fkst-test-bot" }
    assert_errors(function()
      detect_bridge.detect(current, "other/repo", managed)
    end)
  end,
}
