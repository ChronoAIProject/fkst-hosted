local core = require("core")
local t = fkst.test

local function source_ref()
  return {
    kind = "external",
    ref = "owner/repo#issue/42",
  }
end

return {
  test_entity_to_issue_maps_github_issue = function()
    local issue = core.entity_to_issue({
      schema = "github-proxy.v1",
      type = "issue",
      repo = "owner/repo",
      number = 42,
      title = "Bridge issue",
      url = "https://github.example/owner/repo/issues/42",
      state = "OPEN",
      updated_at = "2026-06-03T01:02:03Z",
      source_ref = source_ref(),
      dedup_key = "owner/repo#issue#42@2026-06-03T01:02:03Z",
    })

    t.eq(issue.schema, "autochrono.issue.v1")
    t.eq(issue.repo, "owner/repo")
    t.eq(issue.issue_number, 42)
    t.eq(issue.title, "Bridge issue")
    t.eq(issue.url, "https://github.example/owner/repo/issues/42")
    t.eq(issue.state, "OPEN")
    t.eq(issue.updated_at, "2026-06-03T01:02:03Z")
    t.eq(issue.source_ref.ref, "owner/repo#issue/42")
    t.eq(issue.dedup_key, "owner/repo#issue#42@2026-06-03T01:02:03Z")
  end,

  test_entity_to_issue_rejects_wrong_schema_or_type = function()
    t.raises(function()
      core.entity_to_issue({ schema = "autochrono.issue.v1", type = "issue" })
    end)
    t.raises(function()
      core.entity_to_issue({ schema = "github-proxy.v1", type = "pr" })
    end)
  end,

  test_reply_to_comment_request_maps_payload = function()
    local request = core.reply_to_comment_request({
      schema = "autochrono.reply.v1",
      repo = "owner/repo",
      issue_number = 42,
      body = "Draft reply",
      dedup_key = "autochrono:owner/repo#issue/42",
      source_ref = source_ref(),
    })

    t.eq(request.schema, "github-proxy.v1")
    t.eq(request.repo, "owner/repo")
    t.eq(request.issue_number, 42)
    t.eq(request.body, "Draft reply")
    t.eq(request.dedup_key, "autochrono:owner/repo#issue/42")
    t.eq(request.source_ref.ref, "owner/repo#issue/42")
  end,

  test_reply_to_comment_request_requires_fields = function()
    t.raises(function()
      core.reply_to_comment_request({
        schema = "autochrono.reply.v1",
        repo = "owner/repo",
        issue_number = 42,
        body = "Draft reply",
      })
    end)
  end,
}
