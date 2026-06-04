local core = require("core")
local t = fkst.test

return {
  test_env_command_whitelist = function()
    t.eq(core.read_env_command("FKST_GITHUB_REPO"), 'printf %s "$FKST_GITHUB_REPO"')
    t.raises(function()
      core.read_env_command("HOME")
    end)
  end,

  test_read_env_empty_is_nil = function()
    local value = core.read_env("FKST_GITHUB_REPO", function(_cmd)
      return { stdout = "", stderr = "", exit_code = 0 }
    end)
    t.is_nil(value)
  end,

  test_dedup_and_seen_key = function()
    local key = core.issue_dedup_key("owner/repo", 12, "2026-06-03T01:02:03Z")
    t.eq(key, "owner/repo#12@2026-06-03T01:02:03Z")
  end,

  test_seen_marker_path = function()
    t.eq(
      core.seen_marker_path("/tmp/fkst-runtime", "owner/x#42@2026-06-03T01:02:03Z"),
      "/tmp/fkst-runtime/github-proxy/seen/6f776e65722f7823343240323032362d30362d30335430313a30323a30335a"
    )
  end,

  test_mkdir_p_cmd = function()
    t.eq(core.mkdir_p_cmd("/tmp/body's"), "mkdir -p '/tmp/body'\\''s'")
  end,

  test_comment_marker = function()
    local key = "owner/repo#1@x"
    local marker = core.comment_marker(key)
    t.eq(marker, "<!-- fkst:github-proxy:comment:owner/repo#1@x -->")
    t.is_true(core.has_marker("hello\n" .. marker .. "\n", key))
    t.eq(core.has_marker("hello", key), false)
  end,

  test_parse_issue_list = function()
    local issues = core.parse_issue_list('[{"number":7,"title":"Fix \\"x\\"","url":"https://example.test/7","updatedAt":"2026-06-03T00:00:00Z","state":"OPEN"}]')
    t.eq(#issues, 1)
    t.eq(issues[1].number, 7)
    t.eq(issues[1].title, 'Fix "x"')
    t.eq(issues[1].updated_at, "2026-06-03T00:00:00Z")
    t.eq(issues[1].state, "OPEN")
  end,

  test_gh_commands_are_quoted = function()
    t.eq(
      core.gh_issue_list_cmd("owner/repo"),
      "gh issue list --repo 'owner/repo' --json number,title,updatedAt,url,state"
    )
    t.eq(
      core.gh_issue_view_comments_cmd("owner/repo", 3),
      "gh issue view '3' --repo 'owner/repo' --json comments"
    )
    t.eq(
      core.gh_issue_comment_cmd("owner/repo", 3, "/tmp/body's.md"),
      "gh issue comment '3' --repo 'owner/repo' --body-file '/tmp/body'\\''s.md'"
    )
  end,
}
