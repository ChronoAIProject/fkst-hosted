local core = require("core")
local t = fkst.test

return {
  test_env_command_whitelist = function()
	    t.eq(core.read_env_command("FKST_GITHUB_REPO"), 'printf %s "$FKST_GITHUB_REPO"')
	    t.eq(core.read_env_command("FKST_GITHUB_BOT_LOGIN"), 'printf %s "$FKST_GITHUB_BOT_LOGIN"')
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

  test_entity_cache_key = function()
    local key = core.entity_cache_key("owner/repo", "issue", 12)
    t.eq(key, "github-proxy/issue/owner/repo/12")
  end,

  test_entity_dedup_key = function()
    local key = core.entity_dedup_key("owner/repo", "pr", 12, "2026-06-03T01:02:03Z")
    t.eq(key, "owner/repo#pr#12@2026-06-03T01:02:03Z")
    t.eq(core.issue_dedup_key("owner/repo", 12, "2026-06-03T01:02:03Z"), "owner/repo#issue#12@2026-06-03T01:02:03Z")
  end,

  test_comment_marker = function()
    local key = "owner/repo#1@x"
    local marker = core.comment_marker(key)
    t.eq(marker, "<!-- fkst:github-proxy:comment:owner/repo#1@x -->")
    t.is_true(core.has_marker("hello\n" .. marker .. "\n", key))
    t.eq(core.has_marker("hello", key), false)
  end,

  test_trusted_comment_marker_requires_bot_author = function()
    local key = "owner/repo#1@x"
    local marker = core.comment_marker(key)
    local comments = core.parse_issue_comments(
      '{"comments":[{"body":"'
        .. marker
        .. '","author":{"login":"ordinary-user"}},{"body":"'
        .. marker
        .. '","author":{"login":"fkst-test-bot"}}]}'
    )

    t.eq(core.has_trusted_marker(comments, key, "other-bot"), false)
    t.eq(core.has_trusted_marker(comments, key, "fkst-test-bot"), true)
  end,

  test_current_devloop_state_default_rank_converges_review_conflict_to_fixing = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
    local comments = core.parse_issue_comments(
      '{"comments":[{"body":"<!-- fkst:github-devloop:state:v1 proposal=\\"'
        .. proposal_id
        .. '\\" state=\\"merge-ready\\" version=\\"'
        .. version
        .. '\\" -->","author":{"login":"fkst-test-bot"}},{"body":"<!-- fkst:github-devloop:state:v1 proposal=\\"'
        .. proposal_id
        .. '\\" state=\\"fixing\\" version=\\"'
        .. version
        .. '\\" -->","author":{"login":"fkst-test-bot"}}]}'
    )

    local current = core.current_devloop_state(comments, proposal_id, "fkst-test-bot")
    t.eq(current.state, "fixing")
  end,

  test_current_devloop_state_default_rank_converges_fixing_to_review_meta = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
    local comments = core.parse_issue_comments(
      '{"comments":[{"body":"<!-- fkst:github-devloop:state:v1 proposal=\\"'
        .. proposal_id
        .. '\\" state=\\"fixing\\" version=\\"'
        .. version
        .. '\\" -->","author":{"login":"fkst-test-bot"}},{"body":"<!-- fkst:github-devloop:state:v1 proposal=\\"'
        .. proposal_id
        .. '\\" state=\\"review-meta\\" version=\\"'
        .. version
        .. '\\" -->","author":{"login":"fkst-test-bot"}}]}'
    )

    local current = core.current_devloop_state(comments, proposal_id, "fkst-test-bot")
    t.eq(current.state, "review-meta")
  end,

  test_current_devloop_state_recognizes_merging = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
    local comments = core.parse_issue_comments(
      '{"comments":[{"body":"<!-- fkst:github-devloop:state:v1 proposal=\\"'
        .. proposal_id
        .. '\\" state=\\"merging\\" version=\\"'
        .. version
        .. '\\" -->","author":{"login":"fkst-test-bot"}}]}'
    )

    local current = core.current_devloop_state(comments, proposal_id, "fkst-test-bot")
    t.eq(current.state, "merging")
  end,

  test_current_devloop_state_default_rank_converges_merging_to_merged = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
    local comments = core.parse_issue_comments(
      '{"comments":[{"body":"<!-- fkst:github-devloop:state:v1 proposal=\\"'
        .. proposal_id
        .. '\\" state=\\"merging\\" version=\\"'
        .. version
        .. '\\" -->","author":{"login":"fkst-test-bot"}},{"body":"<!-- fkst:github-devloop:state:v1 proposal=\\"'
        .. proposal_id
        .. '\\" state=\\"merged\\" version=\\"'
        .. version
        .. '\\" -->","author":{"login":"fkst-test-bot"}}]}'
    )

    local current = core.current_devloop_state(comments, proposal_id, "fkst-test-bot")
    t.eq(current.state, "merged")
  end,

  test_parse_entity_list = function()
    local entities = core.parse_entity_list('[{"number":7,"title":"Fix \\"x\\"","url":"https://example.test/7","updatedAt":"2026-06-03T00:00:00Z","state":"OPEN","labels":[{"name":"fkst-dev:enabled"},{"name":"bug"}]}]')
    t.eq(#entities, 1)
    t.eq(entities[1].number, 7)
    t.eq(entities[1].title, 'Fix "x"')
    t.eq(entities[1].updated_at, "2026-06-03T00:00:00Z")
    t.eq(entities[1].state, "OPEN")
    t.eq(#entities[1].labels, 2)
    t.eq(entities[1].labels[1], "fkst-dev:enabled")
    t.eq(entities[1].labels[2], "bug")
  end,

  test_parse_entity_list_accepts_string_labels = function()
    local entities = core.parse_entity_list('[{"number":7,"title":"Fix","url":"https://example.test/7","updatedAt":"2026-06-03T00:00:00Z","state":"OPEN","labels":["one","two"]}]')
    t.eq(#entities[1].labels, 2)
    t.eq(entities[1].labels[1], "one")
    t.eq(entities[1].labels[2], "two")
  end,

  test_parse_entity_list_empty_array = function()
    local entities = core.parse_entity_list("[]")
    t.eq(#entities, 0)
  end,

  test_parse_entity_list_accepts_updated_at = function()
    local entities = core.parse_entity_list('[{"number":8,"title":"Snake case","url":"https://example.test/8","updated_at":"2026-06-03T04:05:06Z","state":"OPEN"}]')
    t.eq(#entities, 1)
    t.eq(entities[1].updated_at, "2026-06-03T04:05:06Z")
    t.eq(core.parse_issue_list("[]")[1], nil)
  end,

  test_gh_commands_are_quoted = function()
    t.eq(
      core.gh_issue_list_cmd("owner/repo"),
      "gh issue list --repo 'owner/repo' --state all --json number,title,updatedAt,url,state,labels"
    )
    t.eq(
      core.gh_pr_list_cmd("owner/repo"),
      "gh pr list --repo 'owner/repo' --state all --json number,title,updatedAt,url,state,labels"
    )
    t.eq(
      core.gh_pr_list_head_cmd("owner/repo", "devloop-owner-repo-42-01HY"),
      "gh pr list --repo 'owner/repo' --head 'devloop-owner-repo-42-01HY' --state open --json number,url,headRefName,baseRefName,state"
    )
    t.eq(
      core.gh_pr_list_head_cmd("owner/repo", "devloop-owner-repo-42-01HY", "dev"),
      "gh pr list --repo 'owner/repo' --head 'devloop-owner-repo-42-01HY' --base 'dev' --state open --json number,url,headRefName,baseRefName,state"
    )
    t.eq(
      core.git_push_branch_cmd("devloop-owner-repo-42-01HY"),
      "git push -u origin 'devloop-owner-repo-42-01HY'"
    )
    t.eq(
      core.git_show_ref_branch_cmd("devloop-owner-repo-42-01HY"),
      "git show-ref --verify refs/heads/'devloop-owner-repo-42-01HY'"
    )
    t.eq(
      core.parse_git_show_ref_head("abc123 refs/heads/devloop-owner-repo-42-01HY\n", "devloop-owner-repo-42-01HY"),
      "abc123"
    )
    t.eq(
      core.parse_git_show_ref_head("abc123 refs/tags/devloop-owner-repo-42-01HY\n", "devloop-owner-repo-42-01HY"),
      nil
    )
    t.eq(
      core.gh_pr_create_cmd("owner/repo", "devloop-owner-repo-42-01HY", nil, "Fix title", "/tmp/body.md"),
      "gh pr create --repo 'owner/repo' --head 'devloop-owner-repo-42-01HY' --title 'Fix title' --body-file '/tmp/body.md'"
    )
    t.eq(
      core.gh_pr_create_cmd("owner/repo", "devloop-owner-repo-42-01HY", "dev", "Fix title", "/tmp/body.md"),
      "gh pr create --repo 'owner/repo' --head 'devloop-owner-repo-42-01HY' --base 'dev' --title 'Fix title' --body-file '/tmp/body.md'"
    )
    local listed = core.parse_pr_list_for_head('[{"number":7,"headRefName":"devloop-owner-repo-42-01HY","baseRefName":"dev","state":"OPEN"}]', "devloop-owner-repo-42-01HY")
    t.eq(listed.number, 7)
    t.eq(listed.base_ref_name, "dev")
    t.eq(core.parse_pr_list_for_head('[{"number":7,"headRefName":"devloop-owner-repo-42-01HY","state":"CLOSED"}]', "devloop-owner-repo-42-01HY"), nil)
    t.eq(
      core.gh_pr_view_head_oid_cmd("owner/repo", 7),
      "gh pr view '7' --repo 'owner/repo' --json headRefOid,baseRefName,state,headRepository,headRepositoryOwner,isCrossRepository"
    )
    local same_repo_pr = core.parse_pr_view_head_state(
      '{"headRefOid":"ABC123","state":"OPEN","headRepository":{"nameWithOwner":"owner/repo"},"isCrossRepository":false}',
      "owner/repo"
    )
    t.eq(same_repo_pr.head_ref_oid, "abc123")
    t.eq(same_repo_pr.state, "OPEN")
    t.eq(same_repo_pr.head_repository, "owner/repo")
    t.eq(same_repo_pr.is_target_repository, true)
    -- Real gh form (observed via dogfood): a merged / branch-deleted PR returns
    -- headRepository.nameWithOwner as an empty string. Fall back to owner/name so
    -- a legitimate same-repo PR is not misjudged as cross-repo.
    local empty_nwo_pr = core.parse_pr_view_head_state(
      '{"headRefOid":"ABC123","state":"MERGED","headRepository":{"name":"fkst-packages","nameWithOwner":""},"headRepositoryOwner":{"login":"ChronoAIProject"},"isCrossRepository":false}',
      "ChronoAIProject/fkst-packages"
    )
    t.eq(empty_nwo_pr.head_repository, "ChronoAIProject/fkst-packages")
    t.eq(empty_nwo_pr.is_target_repository, true)
    t.eq(core.parse_pr_view_head_state(
      '{"headRefOid":"ABC123","state":"OPEN","headRepository":{"nameWithOwner":"fork/repo"},"isCrossRepository":true}',
      "owner/repo"
    ).is_target_repository, false)
    t.eq(core.parse_pr_create("https://example.test/pull/8\n").number, 8)
    t.eq(
      core.gh_issue_view_comments_cmd("owner/repo", 3),
      "gh issue view '3' --repo 'owner/repo' --json comments"
    )
    t.eq(
      core.gh_issue_comment_cmd("owner/repo", 3, "/tmp/body's.md"),
      "gh issue comment '3' --repo 'owner/repo' --body-file '/tmp/body'\\''s.md'"
    )
    t.eq(
      core.gh_issue_edit_labels_cmd("owner/repo", 3, { "fkst-dev:ready" }, { "fkst-dev:thinking", "needs'user" }),
      "gh issue edit '3' --repo 'owner/repo' --add-label 'fkst-dev:ready' --remove-label 'fkst-dev:thinking' --remove-label 'needs'\\''user'"
    )
  end,
}
