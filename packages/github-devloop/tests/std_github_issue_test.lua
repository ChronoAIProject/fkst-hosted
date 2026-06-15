local gh = require("std.github")
local issue_adapter = require("std.github.issue")
local core = require("core")

local function assert_comment_equal(left, right)
  assert(left.body == right.body)
  assert(left.author_login == right.author_login)
  assert(left.created_at == right.created_at)
end

return {
  test_read_issue_builds_exact_generic_command_and_parses_full_issue = function()
    local seen
    local handle = gh.new(function(opts)
      seen = opts.cmd
      return {
        stdout = '{"number":42,"state":"OPEN","title":"t","body":"issue body","url":"https://github.com/owner/repo/issues/42","updatedAt":"2026-06-15T00:00:00Z","labels":[{"name":"fkst-dev:enabled"}],"comments":[{"id":1,"body":"b","author":{"login":"bot"},"createdAt":"2026-06-14T00:00:00Z"}],"assignees":[{"login":"dev"}],"author":{"login":"author"}}',
        stderr = "",
        exit_code = 0,
      }
    end)

    local issue = handle.read_issue({ kind = "external", ref = "owner/repo#issue/42" })

    assert(seen == "gh issue view '42' --repo 'owner/repo' --json number,title,body,url,updatedAt,state,labels,comments,assignees,author")
    assert(issue.number == 42)
    assert(issue.source_ref.kind == "external")
    assert(issue.source_ref.ref == "owner/repo#issue/42")
    assert(issue.title == "t")
    assert(issue.body == "issue body")
    assert(issue.url == "https://github.com/owner/repo/issues/42")
    assert(issue.updated_at == "2026-06-15T00:00:00Z")
    assert(issue.state == "OPEN")
    assert(issue.labels[1] == "fkst-dev:enabled")
    assert(issue.comments[1].id == 1)
    assert(issue.comments[1].body == "b")
    assert(issue.comments[1].author_login == "bot")
    assert(issue.comments[1].created_at == "2026-06-14T00:00:00Z")
    assert(issue.assignees[1] == "dev")
    assert(issue.author_login == "author")
  end,

  test_normalize_issue_preserves_loop_used_fields_from_old_loop_stdout = function()
    local stdout = '{"state":"OPEN","title":"t","updatedAt":"2026-06-15T00:00:00Z","labels":[{"name":"fkst-dev:enabled"},{"name":"bug"}],"comments":[{"id":1,"body":"b","author":{"login":"bot"},"createdAt":"2026-06-14T00:00:00Z"}],"assignees":[{"login":"dev"}],"author":{"login":"author"}}'
    local ref = { kind = "external", ref = "owner/repo#issue/42" }
    local normalized = issue_adapter.normalize_issue(stdout, ref)
    local old = core.parse_issue_view_loop(stdout)

    assert(normalized.title == old.title)
    assert(normalized.updated_at == old.updated_at)
    assert(normalized.state == old.state)
    assert(#normalized.labels == #old.labels)
    for index, label in ipairs(old.labels) do
      assert(normalized.labels[index] == label)
    end
    assert(#normalized.comments == #old.comments)
    for index, comment in ipairs(old.comments) do
      assert_comment_equal(normalized.comments[index], comment)
    end
    assert(#normalized.assignees == #old.assignees)
    for index, assignee in ipairs(old.assignees) do
      assert(normalized.assignees[index] == assignee)
    end
    assert(normalized.author_login == old.author_login)
  end,
}
