local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function assert_runtime_key(key)
  local text = tostring(key or "")
  t.is_true(text ~= "")
  for segment in text:gmatch("[^/]+") do
    t.is_true(segment ~= "")
    t.is_true(segment:find("^[A-Za-z0-9._-]+$") ~= nil)
    t.is_true(segment:find("^%.*$") == nil)
  end
end

local function assert_distinct_keys(keys)
  local seen = {}
  for _, key in ipairs(keys or {}) do
    assert_runtime_key(key)
    t.is_nil(seen[key])
    seen[key] = true
  end
end

local function list_helpers_without_observe_coalesce()
  return {
    core.gh_issue_list_intake_cmd("owner/repo", 100),
    core.gh_issue_list_intake_probe_cmd("owner/repo", 5),
    core.gh_issue_list_decompose_children_cmd("owner/repo", "github-devloop/issue/owner/repo/42"),
    core.gh_issue_list_recent_closed_cmd("owner/repo", 30),
    core.gh_issue_list_wip_cmd("owner/repo"),
    core.gh_dashboard_issue_list_cmd("owner/repo", "fkst-dashboard"),
    core.gh_dashboard_issue_all_open_cmd("owner/repo"),
    core.gh_repo_labels_list_cmd("owner/repo"),
    core.gh_pr_list_freshness_cmd("owner/repo"),
    core.gh_pr_list_merge_queue_cmd("owner/repo", "dev"),
    core.gh_pr_list_head_base_cmd("owner/repo", "integration/dev", "dev"),
    core.gh_pr_list_head_cmd("owner/repo", "integration/dev"),
  }
end

return {
  test_gh_issue_view_state_command_and_parse = function()
    t.eq(
      core.gh_issue_list_intake_cmd("owner/repo", 50),
      "gh issue list --repo 'owner/repo' --state open --limit 50 --json number,title,body,updatedAt,labels,assignees,author"
    )
    t.eq(
      core.gh_issue_list_intake_probe_cmd("owner/repo", 5),
      "gh api 'repos/owner/repo/issues?state=open&sort=created&direction=desc&per_page=5'"
    )
    t.eq(
      core.gh_issue_list_intake_probe_cmd("owner/repo", 5, "2026-06-03T01:02:03Z"),
      "gh api 'repos/owner/repo/issues?state=open&sort=created&direction=desc&per_page=5&since=2026-06-03T01%3A02%3A03Z'"
    )
    t.eq(core.gh_issue_list_observe_cmd("owner/repo"), "gh api --paginate --slurp 'repos/owner/repo/issues?state=open&per_page=100'")
    t.eq(core.gh_issue_list_observe_cmd("owner/repo", core._enabled_label), "gh api --paginate --slurp 'repos/owner/repo/issues?state=open&labels=fkst-dev%3Aenabled&per_page=100'")
    t.eq(core.gh_issue_list_observe_cmd("owner/repo", core._enabled_label, 2), "gh api 'repos/owner/repo/issues?state=open&labels=fkst-dev%3Aenabled&per_page=100&page=2'")
    t.eq(core.gh_pr_list_observe_cmd("owner/repo", 1), "gh api 'repos/owner/repo/pulls?state=open&per_page=100&page=1'")
    local issue_observe_opts = core.gh_exec_opts(core.gh_issue_list_observe_opts("owner/repo", core._enabled_label, 2), 60)
    t.eq(issue_observe_opts.cmd, core.gh_issue_list_observe_cmd("owner/repo", core._enabled_label, 2))
    t.eq(issue_observe_opts.timeout, 10)
    t.eq(issue_observe_opts.read_coalesce.key, "github-devloop/observe-list/v-owner/v-repo/issues/label/v-fkst-dev_3Aenabled/page/2")
    t.eq(issue_observe_opts.read_coalesce.ttl_seconds, 30)
    local pr_observe_opts = core.gh_exec_opts(core.gh_pr_list_observe_opts("owner/repo", 1, true), 60)
    t.eq(pr_observe_opts.cmd, core.gh_pr_list_observe_cmd("owner/repo", 1, true))
    t.eq(pr_observe_opts.timeout, 10)
    t.eq(pr_observe_opts.read_coalesce.key, "github-devloop/observe-list/v-owner/v-repo/prs/page/1")
    t.eq(pr_observe_opts.read_coalesce.ttl_seconds, 30)
    t.eq(
      core.gh_pr_list_head_base_cmd("owner/repo", "integration/dev", "dev"),
      "gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&head=owner%3Aintegration%2Fdev&base=dev&per_page=100'"
    )
    local intake = core.parse_issue_list_intake('[[{"number":42,"title":"Fix","updated_at":"2026-06-03T01:02:03Z","labels":[{"name":"bug"}]}]]')
    t.eq(intake[1].number, 42)
    t.eq(intake[1].body, "")
    t.eq(intake[1].created_at, nil)
    t.eq(intake[1].updated_at, "2026-06-03T01:02:03Z")
    t.eq(intake[1].labels[1], "bug")
    local mixed = core.parse_issue_list_intake('[[{"number":1,"pull_request":{"url":"https://api.example.test/pulls/1"}}],[{"number":2,"title":"Issue","updated_at":"2026-06-03T01:02:04Z","labels":[]}]]', 1)
    t.eq(#mixed, 1)
    t.eq(mixed[1].number, 2)
    t.eq(#core.parse_issue_list_intake("[[]]"), 0)
    t.eq(#core.parse_issue_list_observe("[[]]"), 0)
    t.eq(#core.parse_pr_list_observe("[[]]"), 0)
    t.eq(#core.parse_pr_list_head_base("[[]]"), 0)
    local rollup_prs = core.parse_pr_list_head_base('[[{"number":9,"head":{"sha":"abc123","ref":"integration/dev"},"base":{"ref":"dev"},"state":"open"}]]')
    t.eq(rollup_prs[1].number, 9)
    t.eq(rollup_prs[1].head_sha, "abc123")
    t.eq(rollup_prs[1].head_ref_name, "integration/dev")
    t.eq(rollup_prs[1].base_ref_name, "dev")

    t.eq(
      core.gh_issue_view_state_cmd("owner/repo", 42),
      "gh issue view '42' --repo 'owner/repo' --json title,updatedAt,labels,state,comments,assignees,author"
    )
    t.eq(
      core.gh_issue_view_result_cmd("owner/repo", 42),
      "gh issue view '42' --repo 'owner/repo' --json labels,comments"
    )

    local state = core.parse_issue_view_state('{"updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[{"name":"fkst-dev:enabled"}],"comments":[{"body":"hello","author":{"login":"fkst-test-bot"}}]}')
    t.eq(state.state, "OPEN")
    t.eq(state.updated_at, "2026-06-03T01:02:03Z")
    t.eq(state.labels[1], "fkst-dev:enabled")
    t.eq(core.comment_body(state.comments[1]), "hello")
    t.eq(core.comment_author_login(state.comments[1]), "fkst-test-bot")

    local proposal_id = "github-devloop/issue/owner/repo/42"
    local decision = "approve"
    local dedup_key = "consensus:github-devloop/issue/owner/repo/42/v1"
    local result = core.parse_issue_view_result(
      '{"labels":["fkst-dev:ready"],"comments":[{"body":"'
        .. core.result_marker(proposal_id, decision, dedup_key):gsub('"', '\\"')
        .. '","author":{"login":"fkst-test-bot"}}]}'
    )
    t.eq(core.has_terminal_label(result.labels), true)
    t.eq(core.has_result_marker(result.comments, proposal_id, decision, dedup_key), true)
  end,
  test_observe_list_read_coalesce_keys_are_injective_for_scope_segments = function()
    local keys = {
      core.gh_issue_list_observe_read_coalesce("owner/repo", "a_58_b", 1).key,
      core.gh_issue_list_observe_read_coalesce("owner/repo", "a:b", 1).key,
      core.gh_issue_list_observe_read_coalesce("owner/repo", "a%b", 1).key,
      core.gh_issue_list_observe_read_coalesce("owner/repo", "a_b", 1).key,
      core.gh_issue_list_observe_read_coalesce("owner/repo", "fkst-dev:enabled", 1).key,
      core.gh_issue_list_observe_read_coalesce("owner/repo", nil, 1).key,
      core.gh_issue_list_observe_read_coalesce("owner/repo", "a:b", 2).key,
      core.gh_issue_list_observe_read_coalesce("owner/other", "a:b", 1).key,
      core.gh_issue_list_observe_read_coalesce("owner_repo/name", "a:b", 1).key,
      core.gh_issue_list_observe_read_coalesce("owner/repo.name", "a:b", 1).key,
      core.gh_issue_list_observe_read_coalesce("./repo", "a:b", 1).key,
      core.gh_issue_list_observe_read_coalesce("../repo", "a:b", 1).key,
      core.gh_issue_list_observe_read_coalesce("owner/.", "a:b", 1).key,
      core.gh_issue_list_observe_read_coalesce("owner/..", "a:b", 1).key,
      core.gh_pr_list_observe_read_coalesce("owner/repo", 1).key,
      core.gh_pr_list_observe_read_coalesce("owner/repo", 2).key,
      core.gh_pr_list_observe_read_coalesce("owner/other", 1).key,
    }

    assert_distinct_keys(keys)
    t.eq(core.gh_issue_list_observe_read_coalesce("owner/repo", "a_58_b", 1).key:find("a_58_b", 1, true), nil)
    t.is_true(core.gh_issue_list_observe_read_coalesce("owner/repo", "a:b", 1).key:find("v-a_3Ab", 1, true) ~= nil)
    t.is_true(core.gh_issue_list_observe_read_coalesce("owner/repo", "a_58_b", 1).key
      ~= core.gh_issue_list_observe_read_coalesce("owner/repo", "a:b", 1).key)
  end,

  test_observe_list_read_coalesce_opts_share_timeout = function()
    local specs = {
      core.gh_exec_opts(core.gh_issue_list_observe_opts("owner/repo", core._enabled_label, 1, true), 30),
      core.gh_exec_opts(core.gh_issue_list_observe_opts("owner/repo", core.state_label("ready"), 1, true), 60),
      core.gh_exec_opts(core.gh_issue_list_observe_opts("owner/repo", nil, 1, true), 90),
      core.gh_exec_opts(core.gh_issue_list_observe_opts("owner/repo", core._enabled_label, 2), 30),
      core.gh_exec_opts(core.gh_pr_list_observe_opts("owner/repo", 1, true), 30),
      core.gh_exec_opts(core.gh_pr_list_observe_opts("owner/repo", 2), 60),
      core.gh_exec_opts(core.gh_issue_list_observe_opts("owner/repo", core._enabled_label), 60),
      core.gh_exec_opts(core.gh_pr_list_observe_opts("owner/repo"), 90),
    }

    for _, spec in ipairs(specs) do
      t.eq(spec.timeout, 10)
      t.eq(spec.read_coalesce.ttl_seconds, 30)
      assert_runtime_key(spec.read_coalesce.key)
    end
  end,

  test_non_observe_list_reads_do_not_carry_read_coalesce = function()
    for _, cmd in ipairs(list_helpers_without_observe_coalesce()) do
      t.is_nil(core.gh_exec_opts(cmd, 30).read_coalesce)
    end
  end,

  test_gh_issue_view_commands_match_existing_strings = function()
    local cases = {
      { core.gh_issue_view_intake_scan_cmd, "title,labels,comments,state,assignees,author" },
      { core.gh_issue_view_intake_judge_cmd, "title,body,updatedAt,labels,comments,state,assignees,author" },
      { core.gh_issue_view_state_cmd, "title,updatedAt,labels,state,comments,assignees,author" },
      { core.gh_issue_view_result_cmd, "labels,comments" },
      { core.gh_issue_view_loop_cmd, "title,updatedAt,labels,comments,state" },
      { core.gh_issue_view_meta_cmd, "title,labels,comments" },
      { core.gh_issue_view_implement_cmd, "title,body,labels,comments,state,author" },
      { core.gh_issue_view_open_pr_cmd, "title,labels,comments,assignees,author" },
      { core.gh_issue_view_reviewing_cmd, "labels,comments" },
      { core.gh_issue_view_review_cmd, "title,labels,comments,assignees,author" },
      { core.gh_issue_view_decompose_cmd, "title,body,labels,comments" },
      { core.gh_issue_view_fix_cmd, "title,labels,comments" },
      { core.gh_issue_view_review_loop_cmd, "title,labels,comments,assignees,author" },
      { core.gh_issue_view_merge_cmd, "title,labels,comments,state,assignees" },
      { core.gh_issue_view_observe_cmd, "title,comments,state,stateReason,assignees,author" },
    }

    for _, case in ipairs(cases) do
      t.eq(case[1]("owner/repo", 42), "gh issue view '42' --repo 'owner/repo' --json " .. case[2])
    end
    t.eq(
      core.gh_check_run_rerequest_cmd("owner/repo", 123),
      "gh api --method POST 'repos/owner/repo/check-runs/123/rerequest'"
    )
    t.eq(
      core.gh_issue_list_decompose_children_cmd("owner/repo", "github-devloop/issue/owner/repo/42"),
      "gh issue list --repo 'owner/repo' --state all --limit 100 --search 'fkst:github-devloop:decompose-child:v1 github-devloop/issue/owner/repo/42' --json number,title,state,author,body,url"
    )
  end,
  test_intake_judge_parse_keeps_full_issue_body = function()
    local long_body = string.rep("body-line-", core.max_body_len() + 1) .. "FULL_BODY_TAIL"
    local parsed = core.parse_issue_view_intake_judge(
      '{"title":"Long intake","body":"' .. long_body .. '","updatedAt":"2026-06-03T01:02:03Z","state":"OPEN","labels":[{"name":"bug"}],"comments":[]}'
    )

    t.eq(parsed.title, "Long intake")
    t.eq(parsed.body, long_body)
    t.is_true(#parsed.body > core.max_body_len())
    t.is_true(parsed.body:find("FULL_BODY_TAIL", 1, true) ~= nil)
    t.eq(parsed.updated_at, "2026-06-03T01:02:03Z")
    t.eq(parsed.state, "OPEN")
    t.eq(parsed.labels[1], "bug")
  end,
  test_meta_parse_omits_issue_body_snapshot = function()
    local long_body = string.rep("body-line-", core.max_body_len() + 1) .. "FULL_BODY_TAIL"
    local parsed = core.parse_issue_view_meta(
      '{"title":"Long meta","body":"' .. long_body .. '","labels":[{"name":"bug"}],"comments":[]}'
    )

    t.eq(parsed.title, "Long meta")
    t.is_nil(parsed.body)
    t.eq(parsed.labels[1], "bug")
  end,
  test_decompose_parse_keeps_full_issue_body_for_lineage_only = function()
    local long_body = string.rep("body-line-", core.max_body_len() + 1) .. "FULL_BODY_TAIL"
    local parsed = core.parse_issue_view_decompose(
      '{"title":"Long decompose","body":"' .. long_body .. '","labels":[{"name":"bug"}],"comments":[]}'
    )

    t.eq(parsed.title, "Long decompose")
    t.eq(parsed.body, long_body)
    t.is_true(#parsed.body > core.max_body_len())
    t.is_true(parsed.body:find("FULL_BODY_TAIL", 1, true) ~= nil)
  end,
}
