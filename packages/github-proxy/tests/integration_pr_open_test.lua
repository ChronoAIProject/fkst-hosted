local h = require("tests.proxy_integration_helpers")
local t = h.t
local core = h.core
local issue_list_json = h.issue_list_json
local pr_list_json = h.pr_list_json
local runtime_root = h.runtime_root
local opts = h.opts
local mock_repo_env = h.mock_repo_env
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local mock_issue_list = h.mock_issue_list
local mock_pr_list = h.mock_pr_list
local mock_poll = h.mock_poll
local json_string = h.json_string
local comment_json = h.comment_json
local mock_comment_view = h.mock_comment_view
local mock_comment_view_failure = h.mock_comment_view_failure
local mock_label_view = h.mock_label_view
local mock_pr_open_guard = h.mock_pr_open_guard
local mock_branch_head = h.mock_branch_head
local mock_branch_head_descends = h.mock_branch_head_descends
local mock_non_branch_ref_head = h.mock_non_branch_ref_head
local mock_comment_write = h.mock_comment_write
local mock_label_write = h.mock_label_write
local mock_pr_head_list = h.mock_pr_head_list
local mock_pr_head_state = h.mock_pr_head_state
local mock_git_push = h.mock_git_push
local mock_pr_create = h.mock_pr_create
local mock_pr_create_stdout = h.mock_pr_create_stdout
local mock_pr_comment_view = h.mock_pr_comment_view
local mock_pr_comment_write = h.mock_pr_comment_write
local calls_matching = h.calls_matching
local count_calls = h.count_calls
local long_dedup = h.long_dedup
local pr_open_event = h.pr_open_event
local pr_open_guard_comments = h.pr_open_guard_comments
local pr_open_visible_comments = h.pr_open_visible_comments
local reviewing_marker = h.reviewing_marker
require("tests.entity_view_probe_helpers")

local function count_exact_calls(rendered)
  local total = 0
  for _, call in ipairs(t.command_calls()) do
    if call.rendered == rendered then
      total = total + 1
    end
  end
  return total
end

local function has_arg_pair(rendered, flag, value)
  local text = tostring(rendered or "")
  return text:find(tostring(flag) .. " '" .. tostring(value) .. "'", 1, true) ~= nil
    or text:find(tostring(flag) .. " " .. tostring(value), 1, true) ~= nil
end

local function assert_pr_rest_view_fetch(pr_number)
  t.eq(count_calls("gh api repos/owner/x/pulls/" .. tostring(pr_number)), 1)
  t.is_true(count_calls("gh api --paginate --slurp repos/owner/x/issues/" .. tostring(pr_number) .. "/comments?per_page=100") >= 1)
end

return {
  test_pr_open_request_dry_run_does_not_push_or_create = function()
    mock_write_env("")
    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-dry-run"))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("gh issue comment"), 0)
  end,

  test_pr_open_request_pushes_creates_comments_and_labels = function()
    mock_write_env("1")
    mock_bot_env()
    local event = pr_open_event()
    event.payload.claim = {
      owner = "fkst-test-bot",
      source_ref = event.payload.source_ref,
    }
    mock_pr_open_guard(nil, pr_open_guard_comments(), { "fkst-test-bot" })
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("abc123", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", event, opts("pr-open-write", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git push -u origin"), 1)
    t.eq(count_calls("gh pr create"), 1)
    t.eq(count_calls("gh issue comment"), 1)
    t.eq(count_calls("gh pr comment"), 1)
    t.eq(count_calls("gh issue edit"), 1)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "github_entity_changed")
    t.eq(result.raises[1].payload.type, "pr")
    t.eq(result.raises[1].payload.repo, "owner/x")
    t.eq(result.raises[1].payload.number, 7)
    t.eq(result.raises[1].payload.source, "github_pr_open")
    t.eq(result.raises[1].payload.source_ref.ref, "owner/x#pr/7")
    t.eq(result.raises[2].queue, "github_pr_opened")
    t.eq(result.raises[2].payload.schema, "github-proxy.pr-opened.v1")
    t.eq(result.raises[2].payload.repo, "owner/x")
    t.eq(result.raises[2].payload.issue_number, 42)
    t.eq(result.raises[2].payload.proposal_id, "github-devloop/issue/owner/x/42")
    t.eq(result.raises[2].payload.impl_version, "v1")
    t.eq(result.raises[2].payload.pr_number, 7)
    t.eq(result.raises[2].payload.branch, "devloop-owner-x-42-01HY")
    t.eq(result.raises[2].payload.head_sha, "abc123")
    t.eq(result.raises[2].payload.base_branch, "dev")
    t.eq(result.raises[2].payload.source_ref.ref, "owner/x#pr/7")
    t.eq(count_calls("gh api repos/owner/x/issues/42"), 2)
    assert_pr_rest_view_fetch(7)
    local create = calls_matching("gh pr create")[1]
    t.eq(create.rendered:find("--json", 1, true), nil)
    t.is_true(has_arg_pair(create.rendered, "--base", "dev"))

    local issue_written = file.read("/tmp/fkst-github-proxy-pr-open-owner_x-devloop-owner-x-42-01HY-issue-comment.md")
    t.is_true(issue_written:find("github-devloop PR opened: #7", 1, true) ~= nil)
    t.is_true(issue_written:find('state="pr-open"', 1, true) ~= nil)
    t.is_true(issue_written:find('pr="7"', 1, true) ~= nil)

    local pr_written = file.read("/tmp/fkst-github-proxy-pr-open-owner_x-devloop-owner-x-42-01HY-pr-comment.md")
    t.is_true(pr_written:find("fkst:github-devloop:pr-origin:v1", 1, true) ~= nil)
  end,

  test_pr_open_request_skips_when_issue_claim_is_lost = function()
    mock_write_env("1")
    mock_bot_env()
    local event = pr_open_event()
    event.payload.claim = {
      owner = "fkst-test-bot",
      source_ref = event.payload.source_ref,
    }
    mock_pr_open_guard(nil, pr_open_guard_comments(), { "other-bot" })

    local result = t.run_department("departments/github_pr_open/main.lua", event, opts("pr-open-claim-lost", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("gh issue comment"), 0)
    t.eq(count_calls("gh pr comment"), 0)
  end,

  test_pr_open_request_read_after_write_lag_skips_tail_label_update = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("abc123", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing" }, pr_open_guard_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-read-after-write-lag", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue comment"), 1)
    t.eq(count_calls("gh pr comment"), 1)
    t.eq(count_calls("gh api repos/owner/x/issues/42"), 2)
    assert_pr_rest_view_fetch(7)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_pr_open_request_skips_tail_label_update_when_issue_advanced_to_reviewing = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("abc123", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:reviewing" }, pr_open_visible_comments({ reviewing_marker() }))

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-tail-reviewing", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue comment"), 1)
    t.eq(count_calls("gh pr comment"), 1)
    t.eq(count_calls("gh api repos/owner/x/issues/42"), 2)
    assert_pr_rest_view_fetch(7)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_pr_open_request_applies_tail_label_update_when_issue_is_still_pr_open = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("abc123", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-tail-pr-open", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh issue edit"), 1)
    local edit = calls_matching("gh issue edit")[1]
    t.is_true(has_arg_pair(edit.rendered, "--add-label", "fkst-dev:pr-open"))
    t.is_true(has_arg_pair(edit.rendered, "--remove-label", "fkst-dev:implementing"))
  end,

  test_pr_open_write_guard_bypasses_warm_entity_view_cache = function()
    local run_opts = opts("pr-open-write-guard-fresh", {
      FKST_GITHUB_WRITE = "1",
    })
    mock_write_env("1")
    mock_bot_env()
    t.mock_command("gh api repos/owner/x/issues/42", {
      stdout = '{"title":"Cached","body":"","state":"open","updated_at":"2026-06-03T01:02:03Z","labels":[{"name":"fkst-dev:enabled"}],"assignees":[]}\n',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh api --paginate --slurp repos/owner/x/issues/42/comments?per_page=100", {
      stdout = "[]\n",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("gh api repos/owner/x/issues/42 --jq .updated_at // .updatedAt // \"\"", {
      stdout = "2026-06-03T01:02:03Z\n",
      stderr = "",
      exit_code = 0,
    })
    local cached = t.run_department("departments/test_entity_view_probe/main.lua", {
      queue = "entity_view_probe",
      payload = {
        repo = "owner/x",
        kind = "issue",
        number = 42,
        updated_at = "2026-06-03T01:02:03Z",
        consumer = "cache-warmer",
      },
    }, run_opts)
    t.eq(cached.exit_code, 0)

    local guard_calls_before = count_calls("gh api repos/owner/x/issues/42")
    local pr_create_calls_before = count_calls("gh pr create")
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("abc123", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), run_opts)
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api repos/owner/x/issues/42") - guard_calls_before, 2)
    t.eq(count_calls("gh pr create") - pr_create_calls_before, 1)
  end,

  test_pr_open_request_pushes_without_intent_label = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard({ "fkst-dev:implementing" }, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("abc123", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-write-without-label", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git push -u origin"), 1)
    t.eq(count_calls("gh pr create"), 1)
    t.eq(count_calls("gh issue comment"), 1)
    t.eq(count_calls("gh issue edit"), 1)
  end,

  test_pr_open_request_allows_current_descendant_head = function()
    local event = pr_open_event()
    event.payload.head_sha = "def456"
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("def456")
    mock_branch_head_descends(true)
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("def456", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", event, opts("pr-open-descendant-head", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git show-ref --verify refs/heads"), 1)
    t.eq(count_calls("merge-base --is-ancestor"), 1)
    t.eq(count_calls("git push -u origin"), 1)
    t.eq(count_calls("gh pr create"), 1)
    t.eq(result.raises[2].payload.head_sha, "def456")
  end,

  test_pr_open_request_skips_when_branch_head_is_not_descendant = function()
    local event = pr_open_event()
    event.payload.head_sha = "def456"
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("def456")
    mock_branch_head_descends(false)

    local result = t.run_department("departments/github_pr_open/main.lua", event, opts("pr-open-branch-not-descendant", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git show-ref --verify refs/heads"), 1)
    t.eq(count_calls("merge-base --is-ancestor"), 1)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
  end,

  test_pr_open_request_skips_when_payload_head_does_not_match_current_branch = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("def456")

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-payload-head-stale", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git show-ref --verify refs/heads"), 1)
    t.eq(count_calls("merge-base --is-ancestor"), 0)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
  end,

  test_pr_open_request_skips_when_payload_base_mismatches_implementing_fact = function()
    local event = pr_open_event()
    event.payload.base_branch = "main"
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())

    local result = t.run_department("departments/github_pr_open/main.lua", event, opts("pr-open-base-forged", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git show-ref --verify refs/heads"), 0)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
  end,

  test_pr_open_request_fails_closed_when_implementing_marker_missing_base_branch = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, {
      '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="implementing" version="v1" stage_rank="600" -->',
      '<!-- fkst:github-devloop:implementing:v1 proposal="github-devloop/issue/owner/x/42" dedup="v1" branch="devloop-owner-x-42-01HY" head_sha="abc123" base_sha="abc123" -->',
    })

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-missing-base-branch", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git show-ref --verify refs/heads"), 0)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
  end,

  test_pr_open_request_skips_when_same_named_tag_matches_recorded_head = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_non_branch_ref_head("abc123")

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-same-named-tag", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git show-ref --verify refs/heads"), 1)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
  end,

  test_pr_open_request_resolves_created_pr_with_head_list_when_create_stdout_is_unparseable = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create_stdout("created pull request\n")
    mock_pr_head_list('[{"number":11,"url":"https://github.example/owner/x/pull/11","headRefName":"devloop-owner-x-42-01HY","state":"OPEN"}]\n')
    mock_pr_head_state("abc123", "OPEN", nil, nil, nil, 11)
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-list-after-create", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh api --paginate --slurp repos/owner/x/pulls?state=open&head=owner%3A"), 2)
    t.eq(count_calls("gh pr create"), 1)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "github_entity_changed")
    t.eq(result.raises[1].payload.type, "pr")
    t.eq(result.raises[1].payload.number, 11)
    t.eq(result.raises[2].queue, "github_pr_opened")
    t.eq(result.raises[2].payload.pr_number, 11)
    assert_pr_rest_view_fetch(11)
    local issue_written = file.read("/tmp/fkst-github-proxy-pr-open-owner_x-devloop-owner-x-42-01HY-issue-comment.md")
    t.is_true(issue_written:find("github-devloop PR opened: #11", 1, true) ~= nil)
  end,

  test_pr_open_request_fails_closed_when_created_pr_head_mismatches_recorded_head = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("def456", "OPEN")

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-created-head-mismatch", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 1)
    t.eq(count_calls("git push -u origin"), 1)
    t.eq(count_calls("gh pr create"), 1)
    t.eq(count_calls("gh issue comment"), 0)
    t.eq(count_calls("gh pr comment"), 0)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_pr_open_request_fails_closed_when_created_pr_is_not_open = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("abc123", "CLOSED")

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-created-closed", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 1)
    t.eq(count_calls("git push -u origin"), 1)
    t.eq(count_calls("gh pr create"), 1)
    t.eq(count_calls("gh issue comment"), 0)
    t.eq(count_calls("gh pr comment"), 0)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_pr_open_request_fails_closed_when_created_pr_base_mismatches = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("abc123", "OPEN", "owner/x", false, "main")

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-created-base-mismatch", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 1)
    t.eq(count_calls("git push -u origin"), 1)
    t.eq(count_calls("gh pr create"), 1)
    t.eq(count_calls("gh issue comment"), 0)
    t.eq(count_calls("gh pr comment"), 0)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_pr_open_request_reuses_existing_pr_without_duplicate_create = function()
    local event = pr_open_event()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list('[{"number":9,"url":"https://github.example/owner/x/pull/9","headRefName":"devloop-owner-x-42-01HY","state":"OPEN"}]\n')
    mock_pr_head_state("abc123", "OPEN", nil, nil, nil, 9)
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", event, opts("pr-open-existing", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("gh issue comment"), 1)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "github_entity_changed")
    t.eq(result.raises[1].payload.type, "pr")
    t.eq(result.raises[1].payload.number, 9)
    t.eq(result.raises[2].queue, "github_pr_opened")
    t.eq(result.raises[2].payload.pr_number, 9)
    local issue_written = file.read("/tmp/fkst-github-proxy-pr-open-owner_x-devloop-owner-x-42-01HY-issue-comment.md")
    t.is_true(issue_written:find("github-devloop PR opened: #9", 1, true) ~= nil)
  end,

  test_pr_open_request_fails_closed_when_existing_pr_head_mismatches_recorded_head = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list('[{"number":9,"url":"https://github.example/owner/x/pull/9","headRefName":"devloop-owner-x-42-01HY","state":"OPEN"}]\n')
    mock_pr_head_state("def456", "OPEN", nil, nil, nil, 9)

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-existing-head-mismatch", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 1)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("gh issue comment"), 0)
    t.eq(count_calls("gh pr comment"), 0)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_pr_open_request_fails_closed_when_existing_pr_is_cross_repo = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list('[{"number":9,"url":"https://github.example/owner/x/pull/9","headRefName":"devloop-owner-x-42-01HY","state":"OPEN"}]\n')
    mock_pr_head_state("abc123", "OPEN", "fork/x", true, nil, 9)

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-existing-fork", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 1)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("gh issue comment"), 0)
    t.eq(count_calls("gh pr comment"), 0)
    t.eq(count_calls("gh issue edit"), 0)
  end,

  test_pr_open_request_does_not_reuse_closed_same_head_pr = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard(nil, pr_open_guard_comments())
    mock_branch_head("abc123")
    mock_pr_head_list('[{"number":9,"url":"https://github.example/owner/x/pull/9","headRefName":"devloop-owner-x-42-01HY","state":"CLOSED"}]\n')
    mock_git_push()
    mock_pr_create(10)
    mock_pr_head_state("abc123", "OPEN", nil, nil, nil, 10)
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-closed-head-not-reused", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git push -u origin"), 1)
    t.eq(count_calls("gh pr create"), 1)
    local issue_written = file.read("/tmp/fkst-github-proxy-pr-open-owner_x-devloop-owner-x-42-01HY-issue-comment.md")
    t.is_true(issue_written:find("github-devloop PR opened: #10", 1, true) ~= nil)
  end,

  test_pr_open_retry_after_issue_marker_self_heals_missing_pr_backpointer_and_label = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard({ "fkst-dev:implementing" }, pr_open_visible_comments())
    mock_pr_head_list('[{"number":9,"url":"https://github.example/owner/x/pull/9","headRefName":"devloop-owner-x-42-01HY","state":"OPEN"}]\n')
    mock_pr_head_state("abc123", "OPEN", nil, nil, nil, 9)
    mock_pr_comment_view("existing pr comment without origin")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:implementing" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-self-heal", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git show-ref --verify refs/heads"), 0)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("gh issue comment"), 0)
    t.eq(count_calls("gh pr comment"), 1)
    t.eq(count_calls("gh issue edit"), 1)
    t.eq(count_calls("gh api repos/owner/x/issues/42"), 2)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "github_entity_changed")
    t.eq(result.raises[1].payload.type, "pr")
    t.eq(result.raises[1].payload.number, 9)
    t.eq(result.raises[2].queue, "github_pr_opened")
    t.eq(result.raises[2].payload.pr_number, 9)

    local pr_written = file.read("/tmp/fkst-github-proxy-pr-open-owner_x-devloop-owner-x-42-01HY-pr-comment.md")
    t.is_true(pr_written:find("fkst:github-devloop:pr-origin:v1", 1, true) ~= nil)
    local edit = calls_matching("gh issue edit")[1]
    t.is_true(has_arg_pair(edit.rendered, "--add-label", "fkst-dev:pr-open"))
  end,

  test_pr_open_guard_uses_canonical_rank_so_meta_escalated_implementing_can_open_pr = function()
    local event = pr_open_event()
    event.payload.impl_version = "v1/loop/1"
    event.payload.expected_version = "v1/loop/1"
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard({ "fkst-dev:blocked", "fkst-dev:implementing" }, {
      '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="blocked" version="v1" stage_rank="800" -->',
      '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="implementing" version="v1/loop/1" stage_rank="600" -->',
      '<!-- fkst:github-devloop:implementing:v1 proposal="github-devloop/issue/owner/x/42" dedup="v1/loop/1" branch="devloop-owner-x-42-01HY" head_sha="abc123" base_branch="dev" base_sha="abc123" -->',
    })
    mock_branch_head("abc123")
    mock_pr_head_list("[]\n")
    mock_git_push()
    mock_pr_create(7)
    mock_pr_head_state("abc123", "OPEN")
    mock_comment_view("existing issue comment")
    mock_comment_write()
    mock_pr_comment_view("existing pr comment")
    mock_pr_comment_write()
    mock_pr_open_guard({ "fkst-dev:blocked", "fkst-dev:implementing" }, pr_open_visible_comments())
    mock_label_write()

    local result = t.run_department("departments/github_pr_open/main.lua", event, opts("pr-open-canonical-rank", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git push -u origin"), 1)
    t.eq(count_calls("gh pr create"), 1)
  end,

  test_pr_open_retry_after_reviewing_does_not_revert_issue_label = function()
    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard({ "fkst-dev:reviewing" }, pr_open_visible_comments({
      '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="reviewing" version="v1" stage_rank="675" -->',
    }))
    mock_pr_head_list('[{"number":9,"url":"https://github.example/owner/x/pull/9","headRefName":"devloop-owner-x-42-01HY","state":"OPEN"}]\n')
    mock_pr_head_state("abc123", "OPEN", nil, nil, nil, 9, { {
      body = 'github-devloop implementation PR for issue #42\n\n<!-- fkst:github-devloop:pr-origin:v1 proposal="github-devloop/issue/owner/x/42" issue="42" branch="devloop-owner-x-42-01HY" impl_version="v1" base_branch="dev" -->',
      author_login = "fkst-test-bot",
    } })
    mock_pr_open_guard({ "fkst-dev:reviewing" }, pr_open_visible_comments({
      '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="reviewing" version="v1" stage_rank="675" -->',
    }))

    local result = t.run_department("departments/github_pr_open/main.lua", pr_open_event(), opts("pr-open-reviewing-no-label-regress", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("gh pr comment"), 0)
    t.eq(count_calls("gh issue edit"), 0)
    t.eq(count_calls("gh api repos/owner/x/issues/42"), 2)
  end,

  test_pr_open_retry_after_fixing_suffix_does_not_revert_issue_label = function()
    local event = pr_open_event()
    local base = "ready/consensus-github-devloop/issue/owner/x/42/2026-06-04T01-02-03Z"
    event.payload.impl_version = base .. "/loop/2"
    event.payload.expected_version = event.payload.impl_version
    local comments = pr_open_visible_comments({
      '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="pr-open" version="'
        .. base
        .. '/loop/2" stage_rank="650" -->',
      '<!-- fkst:github-devloop:state:v1 proposal="github-devloop/issue/owner/x/42" state="fixing" version="'
        .. base
        .. '/loop/2/fix/1" stage_rank="700" -->',
    })

    mock_write_env("1")
    mock_bot_env()
    mock_pr_open_guard({ "fkst-dev:pr-open" }, comments)
    mock_pr_head_list('[{"number":9,"url":"https://github.example/owner/x/pull/9","headRefName":"devloop-owner-x-42-01HY","state":"OPEN"}]\n')
    mock_pr_head_state("abc123", "OPEN", nil, nil, nil, 9, { {
      body = event.payload.body,
      author_login = "fkst-test-bot",
    } })
    mock_pr_open_guard({ "fkst-dev:pr-open" }, comments)

    local result = t.run_department("departments/github_pr_open/main.lua", event, opts("pr-open-fixing-suffix-no-label-regress", {
      FKST_GITHUB_WRITE = "1",
    }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("git push -u origin"), 0)
    t.eq(count_calls("gh pr create"), 0)
    t.eq(count_calls("gh pr comment"), 0)
    t.eq(count_calls("gh issue edit"), 0)
    t.eq(count_calls("gh api repos/owner/x/issues/42"), 2)
  end,
}
