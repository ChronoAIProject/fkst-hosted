local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

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
      "gh issue view '42' --repo 'owner/repo' --json title,labels,state,comments,assignees,author"
    )
    t.eq(
      core.gh_issue_view_result_cmd("owner/repo", 42),
      "gh issue view '42' --repo 'owner/repo' --json labels,comments"
    )

    local state = core.parse_issue_view_state('{"state":"OPEN","labels":[{"name":"fkst-dev:enabled"}],"comments":[{"body":"hello","author":{"login":"fkst-test-bot"}}]}')
    t.eq(state.state, "OPEN")
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
  test_gh_issue_view_commands_match_existing_strings = function()
    local cases = {
      { core.gh_issue_view_intake_scan_cmd, "title,labels,comments,state,assignees,author" },
      { core.gh_issue_view_intake_judge_cmd, "title,body,updatedAt,labels,comments,state,assignees,author" },
      { core.gh_issue_view_state_cmd, "title,labels,state,comments,assignees,author" },
      { core.gh_issue_view_result_cmd, "labels,comments" },
      { core.gh_issue_view_loop_cmd, "title,updatedAt,labels,comments,state" },
      { core.gh_issue_view_meta_cmd, "title,labels,comments" },
      { core.gh_issue_view_implement_cmd, "title,labels,comments" },
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
      core.gh_workflow_dispatch_ci_cmd("owner/repo", "devloop-owner-repo-42-01HY"),
      "gh workflow run 'ci.yml' --repo 'owner/repo' --ref 'devloop-owner-repo-42-01HY'"
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
