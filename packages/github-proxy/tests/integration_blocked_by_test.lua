local h = require("tests.proxy_integration_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local count_calls = h.count_calls

local function event(extra)
  local payload = {
    schema = "github-proxy.issue-blocked-by.v1",
    repo = "owner/x",
    blocked_issue_number = 42,
    blocking_issue_number = 99,
    dedup_key = "github-devloop/fork/owner/x/issue/42/v1/blocked-by",
    source_ref = {
      kind = "external",
      ref = "owner/x#issue/42",
    },
  }
  for key, value in pairs(extra or {}) do
    payload[key] = value
  end
  return {
    queue = "github_issue_blocked_by_request",
    payload = payload,
  }
end

local function blocked_by_json(nodes)
  local rendered = {}
  for _, node in ipairs(nodes or {}) do
    table.insert(rendered, string.format(
      '{"number":%d,"repository":{"nameWithOwner":"%s"}}',
      node.number,
      h.json_string(node.repo or "owner/x")
    ))
  end
  return '{"data":{"repository":{"issue":{"blockedBy":{"totalCount":'
    .. tostring(#rendered)
    .. ',"pageInfo":{"hasNextPage":false},"nodes":['
    .. table.concat(rendered, ",")
    .. "]}}}}}\n"
end

local function mock_blocked_comments(comments)
  h.mock_comment_view(comments or {})
end

local function mock_blocked_by(nodes)
  t.mock_command(core.gh_issue_blocked_by_cmd("owner/x", 42), {
    stdout = blocked_by_json(nodes or {}),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_node_ids()
  t.mock_command(core.gh_issue_node_id_cmd("owner/x", 42), {
    stdout = '{"id":"I_blocked"}\n',
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(core.gh_issue_node_id_cmd("owner/x", 99), {
    stdout = '{"id":"I_blocking"}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_add_blocked_by()
  t.mock_command(core.gh_add_blocked_by_cmd("I_blocked", "I_blocking"), {
    stdout = '{"data":{"addBlockedBy":{"clientMutationId":null}}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_blocked_by_marker_comment()
  t.mock_command("gh issue comment '42' --repo 'owner/x' --body-file '/tmp/fkst-github-proxy-blocked-by-", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_blocked_by_dry_run_logs_without_mutation = function()
    mock_write_env("")

    local result = t.run_department("departments/github_issue_blocked_by/main.lua", event(), opts("blocked-by-dry-run"))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("addBlockedBy"), 0)
    t.eq(count_calls("gh issue view"), 0)
  end,

  test_blocked_by_real_write_adds_graphql_edge_and_marker = function()
    mock_write_env("1")
    mock_bot_env()
    mock_blocked_comments({})
    mock_blocked_by({})
    mock_node_ids()
    mock_add_blocked_by()
    mock_blocked_by_marker_comment()

    local result = t.run_department("departments/github_issue_blocked_by/main.lua", event(), opts("blocked-by-real", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("addBlockedBy"), 1)
    t.eq(count_calls("fkst-github-proxy-blocked-by-"), 1)
  end,

  test_blocked_by_existing_edge_skips_mutation_but_writes_marker = function()
    mock_write_env("1")
    mock_bot_env()
    mock_blocked_comments({})
    mock_blocked_by({ { repo = "owner/x", number = 99 } })
    mock_blocked_by_marker_comment()

    local result = t.run_department("departments/github_issue_blocked_by/main.lua", event(), opts("blocked-by-existing-edge", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("addBlockedBy"), 0)
    t.eq(count_calls("gh issue comment '42'"), 1)
  end,

  test_blocked_by_existing_trusted_marker_is_idempotent_noop = function()
    mock_write_env("1")
    mock_bot_env()
    local payload = event().payload
    mock_blocked_comments({
      {
        body = core.blocked_by_marker(payload.dedup_key, payload.blocked_issue_number, payload.blocking_issue_number),
        author_login = "fkst-test-bot",
      },
    })

    local result = t.run_department("departments/github_issue_blocked_by/main.lua", event(), opts("blocked-by-existing-marker", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 0)
    t.eq(count_calls("addBlockedBy"), 0)
    t.eq(count_calls("gh issue comment '42'"), 0)
  end,

  test_blocked_by_malformed_graphql_read_fails_closed_without_effects = function()
    mock_write_env("1")
    mock_bot_env()
    mock_blocked_comments({})
    t.mock_command(core.gh_issue_blocked_by_cmd("owner/x", 42), {
      stdout = '{"data":{"repository":{"issue":{"blockedBy":{"totalCount":1,"pageInfo":{"hasNextPage":false},"nodes":[',
      stderr = "",
      exit_code = 0,
    })

    local result = t.run_department("departments/github_issue_blocked_by/main.lua", event(), opts("blocked-by-malformed-graphql", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 1)
    t.eq(count_calls("addBlockedBy"), 0)
    t.eq(count_calls("gh issue comment '42'"), 0)
  end,

  test_blocked_by_malformed_payload_fails_closed = function()
    mock_write_env("1")

    local result = t.run_department("departments/github_issue_blocked_by/main.lua", event({
      blocked_issue_number = nil,
    }), opts("blocked-by-malformed", {
      FKST_GITHUB_WRITE = "1",
    }))

    t.eq(result.exit_code, 1)
    t.eq(count_calls("addBlockedBy"), 0)
  end,
}
