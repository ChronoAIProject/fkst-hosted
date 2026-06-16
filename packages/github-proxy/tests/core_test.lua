local core = require("core")
local t = fkst.test

return {
  test_env_command_whitelist = function()
	    t.eq(core.read_env_command("FKST_GITHUB_REPO"), 'printf %s "$FKST_GITHUB_REPO"')
	    t.eq(core.read_env_command("FKST_GITHUB_BOT_LOGIN"), 'printf %s "$FKST_GITHUB_BOT_LOGIN"')
	    t.eq(core.read_env_command("FKST_DEVLOOP_REPLAY_BUDGET"), 'printf %s "$FKST_DEVLOOP_REPLAY_BUDGET"')
	    t.eq(core.read_env_command("FKST_DEBUG_STAMP"), 'printf %s "$FKST_DEBUG_STAMP"')
	    t.raises(function()
	      core.read_env_command("HOME")
	    end)
	  end,

  test_github_debug_stamp_is_disabled_by_default = function()
    t.mock_command('printf %s "$FKST_DEBUG_STAMP"', { stdout = "" })

    local body = "Visible reply\n\n<!-- fkst:github-proxy:comment:dedup-1 -->\n"
    local stamped = core.with_github_debug_stamp(body, {
      emitter = "github-proxy.comment",
      target = "issue:owner/repo#42",
      dedup_key = "raw/dedup/1",
      context = "raw context",
    })

    t.eq(stamped, body)
  end,

  test_github_debug_stamp_appends_redacted_metadata_once = function()
    t.mock_command('printf %s "$FKST_DEBUG_STAMP"', { stdout = "1" })
    t.mock_command("git rev-parse --verify HEAD", {
      stdout = "ABCDEF1234567890\n",
      stderr = "",
      exit_code = 0,
    })

    local body = "Visible reply\n\n<!-- fkst:github-proxy:comment:dedup-1 -->\n"
    local stamped = core.with_github_debug_stamp(body, {
      emitter = "github-proxy.comment",
      target = "issue:owner/repo#42",
      dedup_key = "secret/dedup/value",
      context = "secret context value",
    })
    local stamped_again = core.with_github_debug_stamp(stamped, {
      emitter = "github-proxy.comment",
      target = "issue:owner/repo#42",
      dedup_key = "secret/dedup/value",
      context = "secret context value",
    })

    t.eq(stamped_again, stamped)
    t.is_true(stamped:find("Visible reply", 1, true) ~= nil)
    t.is_true(stamped:find("<!-- fkst:github-proxy:comment:dedup-1 -->", 1, true) ~= nil)
    t.is_true(stamped:find("<!-- fkst:debug-stamp:v1", 1, true) ~= nil)
    t.is_true(stamped:find('emitter="github-proxy.comment"', 1, true) ~= nil)
    t.is_true(stamped:find('target="issue:owner/repo#42"', 1, true) ~= nil)
    t.is_true(stamped:find('code_version="abcdef1234567890"', 1, true) ~= nil)
    t.is_true(stamped:find('dedup_hash="', 1, true) ~= nil)
    t.is_true(stamped:find('context_hash="', 1, true) ~= nil)
    t.is_nil(stamped:find("secret/dedup/value", 1, true))
    t.is_nil(stamped:find("secret context value", 1, true))
  end,

  test_read_env_empty_is_nil = function()
    local value = core.read_env("FKST_GITHUB_REPO", function(_cmd)
      return { stdout = "", stderr = "", exit_code = 0 }
    end)
    t.is_nil(value)
  end,

  test_devloop_replay_budget_defaults_to_ten = function()
    local value = core.devloop_replay_budget(function(_cmd)
      return { stdout = "", stderr = "", exit_code = 0 }
    end)
    t.eq(value, 10)
  end,

  test_devloop_replay_budget_parses_bounded_positive_integer = function()
    local value = core.devloop_replay_budget(function(cmd)
      t.eq(cmd, 'printf %s "$FKST_DEVLOOP_REPLAY_BUDGET"')
      return { stdout = " 7 ", stderr = "", exit_code = 0 }
    end)
    t.eq(value, 7)
  end,

  test_devloop_replay_budget_rejects_invalid_values = function()
    t.raises(function()
      core.devloop_replay_budget(function(_cmd)
        return { stdout = "0", stderr = "", exit_code = 0 }
      end)
    end)
    t.raises(function()
      core.devloop_replay_budget(function(_cmd)
        return { stdout = "101", stderr = "", exit_code = 0 }
      end)
    end)
    t.raises(function()
      core.devloop_replay_budget(function(_cmd)
        return { stdout = "1.5", stderr = "", exit_code = 0 }
      end)
    end)
  end,

  test_entity_cache_key = function()
    local key = core.entity_cache_key("owner/repo", "issue", 12)
    t.eq(key, "github-proxy/issue/owner/repo/12")
  end,

  test_entity_view_cache_key = function()
    local key = core.entity_view_cache_key("owner/repo", "issue", 12, "2026-06-03T01:02:03Z")
    t.eq(key, "github-proxy/view/owner/repo/issue/12/2026-06-03T01-02-03Z")
    t.eq(
      core.gh_issue_view_entity_cmd("owner/repo", 12),
      "gh api 'repos/owner/repo/issues/12'"
    )
    t.eq(
      core.gh_pr_view_entity_cmd("owner/repo", 7),
      "gh api 'repos/owner/repo/pulls/7'"
    )
  end,

  test_rest_issue_view_adapter_maps_gh_view_shape = function()
    local adapted = core.rest_issue_to_view_json(
      '{"title":"Issue","body":"Body","state":"open","updated_at":"2026-06-03T01:02:03Z","labels":[{"name":"bug"}],"assignees":[{"login":"fkst-test-bot"}]}',
      '[[{"id":1,"body":"first","user":{"login":"a"}},{"id":2,"body":"second","user":{"login":"b"}}],[{"id":3,"body":"third","user":{"login":"c"}}]]'
    )
    local state = core.parse_issue_state(adapted)
    t.eq(state.labels[1], "bug")
    t.eq(state.assignees[1], "fkst-test-bot")
    t.eq(#state.comments, 3)
    t.eq(state.comments[1].body, "first")
    t.eq(state.comments[2].body, "second")
    t.eq(state.comments[3].body, "third")
    local decoded = json.decode(adapted)
    t.eq(decoded.state, "OPEN")
    t.eq(decoded.updatedAt, "2026-06-03T01:02:03Z")
  end,

  test_rest_view_adapter_escapes_json_control_bytes = function()
    local adapted = core.rest_issue_to_view_json(
      '{"title":"Issue","body":"control\\u0001byte","state":"open","updated_at":"2026-06-03T01:02:03Z","labels":[],"assignees":[]}',
      '[[{"id":1,"body":"comment\\u0002byte","user":{"login":"a"}}]]'
    )
    local decoded = json.decode(adapted)
    t.eq(decoded.body, "control" .. string.char(1) .. "byte")
    t.eq(decoded.comments[1].body, "comment" .. string.char(2) .. "byte")
  end,

  test_rest_issue_view_fails_closed_on_malformed_success_stdout = function()
    t.mock_command(core.gh_issue_rest_view_cmd("owner/repo", 3), {
      stdout = '{"title":',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_comments_api_cmd("owner/repo", 3), {
      stdout = "[]",
      stderr = "",
      exit_code = 0,
    })

    local result = core.fetch_rest_issue_view("owner/repo", 3)
    t.is_true(result.exit_code ~= 0)
    t.eq(result.stdout, "")
    t.is_true(result.stderr:find("github%-proxy: REST response is not valid JSON") ~= nil)
  end,

  test_rest_issue_view_fails_closed_on_empty_success_stdout = function()
    t.mock_command(core.gh_issue_rest_view_cmd("owner/repo", 3), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_comments_api_cmd("owner/repo", 3), {
      stdout = "[]",
      stderr = "",
      exit_code = 0,
    })

    local result = core.fetch_rest_issue_view("owner/repo", 3)
    t.is_true(result.exit_code ~= 0)
    t.eq(result.stdout, "")
    t.is_true(result.stderr:find("github%-proxy: REST entity response is empty") ~= nil)
  end,

  test_rest_pr_view_fails_closed_on_malformed_success_stdout = function()
    t.mock_command(core.gh_pr_rest_view_cmd("owner/repo", 7), {
      stdout = "not json",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_comments_api_cmd("owner/repo", 7), {
      stdout = "[]",
      stderr = "",
      exit_code = 0,
    })

    local result = core.fetch_rest_pr_view("owner/repo", 7)
    t.is_true(result.exit_code ~= 0)
    t.eq(result.stdout, "")
    t.is_true(result.stderr:find("github%-proxy: REST response is not valid JSON") ~= nil)
  end,

  test_rest_pr_view_fails_closed_on_empty_success_stdout = function()
    t.mock_command(core.gh_pr_rest_view_cmd("owner/repo", 7), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_comments_api_cmd("owner/repo", 7), {
      stdout = "[]",
      stderr = "",
      exit_code = 0,
    })

    local result = core.fetch_rest_pr_view("owner/repo", 7)
    t.is_true(result.exit_code ~= 0)
    t.eq(result.stdout, "")
    t.is_true(result.stderr:find("github%-proxy: REST entity response is empty") ~= nil)
  end,

  test_rest_issue_view_empty_comments_stdout_uses_empty_comments_fallback = function()
    t.mock_command(core.gh_issue_rest_view_cmd("owner/repo", 4), {
      stdout = '{"title":"Issue","body":"Body","state":"open","updated_at":"2026-06-03T01:02:03Z","labels":[],"assignees":[]}',
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(core.gh_issue_comments_api_cmd("owner/repo", 4), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })

    local result = core.fetch_rest_issue_view("owner/repo", 4)
    t.eq(result.exit_code, 0)
    local decoded = json.decode(result.stdout)
    local state = core.parse_issue_state(result.stdout)
    t.eq(decoded.title, "Issue")
    t.eq(#state.comments, 0)
  end,

  test_rest_pr_view_adapter_maps_states_and_repository_facts = function()
    local open = core.rest_pr_to_view_json(
      '{"head":{"ref":"branch","sha":"ABC123","repo":{"full_name":"owner/repo","owner":{"login":"owner"}}},"base":{"ref":"dev","repo":{"full_name":"owner/repo","owner":{"login":"owner"}}},"state":"open","updated_at":"2026-06-03T02:03:04Z"}',
      '[[{"id":1,"body":"hello","user":{"login":"fkst-test-bot"}}]]'
    )
    local parsed_open = core.parse_pr_view_head_state(open, "owner/repo")
    t.eq(parsed_open.head_ref_oid, "abc123")
    t.eq(parsed_open.base_ref_name, "dev")
    t.eq(parsed_open.state, "OPEN")
    t.eq(parsed_open.head_repository, "owner/repo")
    t.eq(parsed_open.is_cross_repository, false)
    t.eq(parsed_open.is_target_repository, true)
    t.eq(#core.parse_issue_comments(open), 1)

    local merged = core.parse_pr_view_head_state(core.rest_pr_to_view_json(
      '{"head":{"ref":"branch","sha":"ABC123","repo":{"full_name":"owner/repo","owner":{"login":"owner"}}},"base":{"ref":"dev","repo":{"full_name":"owner/repo","owner":{"login":"owner"}}},"state":"closed","merged":true}',
      "[]"
    ), "owner/repo")
    t.eq(merged.state, "MERGED")

    local open_with_null_merged_at = core.parse_pr_view_head_state(core.rest_pr_to_view_json(
      '{"head":{"ref":"branch","sha":"ABC123","repo":{"full_name":"owner/repo","owner":{"login":"owner"}}},"base":{"ref":"dev","repo":{"full_name":"owner/repo","owner":{"login":"owner"}}},"state":"open","merged":false,"merged_at":null}',
      "[]"
    ), "owner/repo")
    t.eq(open_with_null_merged_at.state, "OPEN")

    local closed_with_null_merged_at = core.parse_pr_view_head_state(core.rest_pr_to_view_json(
      '{"head":{"ref":"branch","sha":"ABC123","repo":{"full_name":"owner/repo","owner":{"login":"owner"}}},"base":{"ref":"dev","repo":{"full_name":"owner/repo","owner":{"login":"owner"}}},"state":"closed","merged":false,"merged_at":null}',
      "[]"
    ), "owner/repo")
    t.eq(closed_with_null_merged_at.state, "CLOSED")

    local merged_at_string = core.parse_pr_view_head_state(core.rest_pr_to_view_json(
      '{"head":{"ref":"branch","sha":"ABC123","repo":{"full_name":"owner/repo","owner":{"login":"owner"}}},"base":{"ref":"dev","repo":{"full_name":"owner/repo","owner":{"login":"owner"}}},"state":"closed","merged":false,"merged_at":"2026-06-03T03:04:05Z"}',
      "[]"
    ), "owner/repo")
    t.eq(merged_at_string.state, "MERGED")

    local fork = core.parse_pr_view_head_state(core.rest_pr_to_view_json(
      '{"head":{"ref":"branch","sha":"ABC123","repo":{"full_name":"fork/repo","owner":{"login":"fork"}}},"base":{"ref":"dev","repo":{"full_name":"owner/repo","owner":{"login":"owner"}}},"state":"open"}',
      "[]"
    ), "owner/repo")
    t.eq(fork.is_cross_repository, true)
    t.eq(fork.is_target_repository, false)

    local deleted_ok = pcall(function()
      core.rest_pr_to_view_json(
        '{"head":{"ref":"branch","sha":"ABC123","repo":null},"base":{"ref":"dev","repo":{"full_name":"owner/repo","owner":{"login":"owner"}}},"state":"open"}',
        "[]"
      )
    end)
    t.eq(deleted_ok, false)
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

  test_trusted_comment_marker_accepts_github_app_bot_suffix = function()
    -- A GitHub App authored the marker: the REST read path reports a
    -- "<slug>[bot]" login, but FKST_GITHUB_BOT_LOGIN holds the bare GraphQL
    -- slug. The author-login normalization must let the bare login match.
    local key = "owner/repo#1@x"
    local marker = core.comment_marker(key)
    local comments = core.parse_issue_comments(
      '{"comments":[{"body":"'
        .. marker
        .. '","author":{"login":"fkst-test-bot[bot]"}}]}'
    )

    t.eq(core.has_trusted_marker(comments, key, "fkst-test-bot"), true)
    t.eq(core.has_trusted_marker(comments, key, "other-bot"), false)
  end,

  test_configure_trusted_bot_login_normalizes_app_bot_suffix = function()
    -- A deployment may configure FKST_GITHUB_BOT_LOGIN as the REST "<slug>[bot]"
    -- form (e.g. a GitHub App) rather than the bare GraphQL slug. It must
    -- normalize to the bare slug so it keeps matching the equally normalized
    -- author logins; bare configs are unaffected.
    t.eq(core.configure_trusted_bot_login("fkst-test-bot[bot]"), "fkst-test-bot")
    t.eq(core.configure_trusted_bot_login("fkst-test-bot"), "fkst-test-bot")

    -- End to end: a "[bot]"-suffixed config still trusts a "[bot]" REST author
    -- (the pre-existing behaviour for such deployments is preserved).
    local key = "owner/repo#1@x"
    local marker = core.comment_marker(key)
    local comments = core.parse_issue_comments(
      '{"comments":[{"body":"' .. marker .. '","author":{"login":"fkst-test-bot[bot]"}}]}'
    )
    t.eq(core.has_trusted_marker(comments, key, core.configure_trusted_bot_login("fkst-test-bot[bot]")), true)

    core.configure_trusted_bot_login("") -- reset shared state to its initial nil
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

  test_current_devloop_state_trailing_fix_suffix_keeps_loop_round = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local base = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-04T01-02-03Z"
    local comments = core.parse_issue_comments(
      '{"comments":[{"body":"<!-- fkst:github-devloop:state:v1 proposal=\\"'
        .. proposal_id
        .. '\\" state=\\"pr-open\\" version=\\"'
        .. base
        .. '/loop/2\\" stage_rank=\\"650\\" -->","author":{"login":"fkst-test-bot"}},{"body":"<!-- fkst:github-devloop:state:v1 proposal=\\"'
        .. proposal_id
        .. '\\" state=\\"reviewing\\" version=\\"'
        .. base
        .. '/loop/2\\" stage_rank=\\"675\\" -->","author":{"login":"fkst-test-bot"}},{"body":"<!-- fkst:github-devloop:state:v1 proposal=\\"'
        .. proposal_id
        .. '\\" state=\\"fixing\\" version=\\"'
        .. base
        .. '/loop/2/fix/1\\" stage_rank=\\"700\\" -->","author":{"login":"fkst-test-bot"}}]}'
    )

    local current = core.current_devloop_state(comments, proposal_id, "fkst-test-bot")
    t.eq(current.state, "fixing")
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
    local entities = core.parse_entity_list('[[{"number":7,"title":"Fix \\"x\\"","html_url":"https://example.test/7","updated_at":"2026-06-03T00:00:00Z","state":"open","labels":[{"name":"fkst-dev:enabled"},{"name":"bug"}]}]]')
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
    local entities = core.parse_entity_list('[[{"number":7,"title":"Fix","html_url":"https://example.test/7","updated_at":"2026-06-03T00:00:00Z","state":"open","labels":["one","two"]}]]')
    t.eq(#entities[1].labels, 2)
    t.eq(entities[1].labels[1], "one")
    t.eq(entities[1].labels[2], "two")
  end,

  test_parse_entity_list_empty_array = function()
    local entities = core.parse_entity_list("[]")
    t.eq(#entities, 0)
  end,

  test_parse_entity_list_empty_slurped_page = function()
    local entities = core.parse_entity_list("[[]]")
    t.eq(#entities, 0)
  end,

  test_parse_entity_list_skips_malformed_rest_items = function()
    local entities = core.parse_entity_list('[[{},{"number":7,"title":"Fix","html_url":"https://example.test/7"}]]')
    t.eq(#entities, 1)
    t.eq(entities[1].number, 7)
  end,

  test_parse_entity_list_accepts_updated_at = function()
    local entities = core.parse_entity_list('[{"number":8,"title":"Snake case","url":"https://example.test/8","updated_at":"2026-06-03T04:05:06Z","state":"OPEN"}]')
    t.eq(#entities, 1)
    t.eq(entities[1].updated_at, "2026-06-03T04:05:06Z")
    t.eq(core.parse_issue_list("[]")[1], nil)
  end,

  test_parse_issue_list_skips_rest_pull_request_shadows = function()
    local entities = core.parse_issue_list('[[{"number":8,"title":"Issue","html_url":"https://example.test/issues/8","updated_at":"2026-06-03T04:05:06Z","state":"open"},{"number":9,"title":"PR","html_url":"https://example.test/pull/9","updated_at":"2026-06-03T04:05:07Z","state":"open","pull_request":{"url":"https://api.example.test/pulls/9"}}]]')
    t.eq(#entities, 1)
    t.eq(entities[1].number, 8)
  end,

  test_gh_exec_returns_success_result = function()
    local result = core.gh_exec("gh issue list", 30, "gh issue list", function(spec)
      t.eq(spec.cmd, "gh issue list")
      t.eq(spec.timeout, 30)
      t.eq(spec.rate_pool.name, "gh")
      t.eq(spec.rate_pool.burst, nil)
      t.eq(spec.rate_pool.refill_per_hour, nil)
      return { stdout = "[]\n", stderr = "", exit_code = 0 }
    end)

    t.eq(result.stdout, "[]\n")
  end,

  test_gh_exec_opts_preserves_options = function()
    local spec = core.gh_exec_opts({ cmd = "gh pr list", timeout = 60, cwd = "/tmp" })
    t.eq(spec.cmd, "gh pr list")
    t.eq(spec.timeout, 60)
    t.eq(spec.cwd, "/tmp")
    t.eq(spec.rate_pool.name, "gh")
  end,

  test_gh_error_classifies_rate_limit_and_abuse = function()
    local api_limit = { stdout = "", stderr = "API rate limit exceeded", exit_code = 1 }
    -- Regression (#710 Finding 1): the dominant "already exceeded" wording was
    -- missed by a contiguous "api rate limit exceeded" needle.
    local already_exceeded = { stdout = "", stderr = "GraphQL: API rate limit already exceeded for user ID 1593871", exit_code = 1 }
    local too_quick = { stdout = "", stderr = "You have triggered an abuse detection mechanism. The request was submitted too quickly.", exit_code = 1 }
    local too_many = { stdout = "", stderr = "HTTP 429: too many requests", exit_code = 1 }

    t.eq(core.is_gh_rate_limited(api_limit), true)
    t.eq(core.is_gh_rate_limited(already_exceeded), true)
    t.eq(core.is_gh_rate_limited(too_quick), true)
    t.eq(core.is_gh_rate_limited(too_many), true)
    t.eq(core.gh_error_class(api_limit), "gh-rate-limited")
    t.eq(core.gh_error("gh issue list", api_limit).class, "gh-rate-limited")
    t.eq(core.gh_error("gh issue list", api_limit).retryable, true)
    t.is_true(core.gh_error_message("gh issue list", api_limit):find("gh-rate-limited", 1, true) ~= nil)
  end,

  test_gh_exec_result_returns_structured_failure = function()
    local ok, err = core.gh_exec_result("gh issue list", 30, "gh issue list", function(_spec)
      return { stdout = "", stderr = "GraphQL: field does not exist", exit_code = 1 }
    end)

    t.eq(ok, false)
    t.eq(err.class, "gh-command-failed")
    t.eq(err.retryable, false)
    t.is_true(err.message:find("gh-command-failed", 1, true) ~= nil)
  end,

  test_error_fact_fields_include_available_delivery_context = function()
    local fields = core.error_fact_fields(
      "gh-command-failed",
      "github_issue_comment_request",
      "github_comment",
      "github-proxy: gh issue comment failed: gh-command-failed: bad sha abcdef1234567890 at 2026-06-10T01:02:03Z /tmp/fkst-a",
      {
        source_ref = { kind = "external", ref = "owner/repo#issue/42" },
        attempt = 2,
        terminal = false,
      }
    )

    t.eq(fields[1], "error_class=gh-command-failed")
    t.eq(fields[2], "fingerprint=" .. core.error_fingerprint(
      "gh-command-failed",
      "github_issue_comment_request",
      "github_comment",
      "github-proxy: gh issue comment failed: gh-command-failed: bad sha fedcba0987654321 at 2026-07-11T09:08:07Z /tmp/fkst-b"
    ))
    t.eq(fields[3], "source_ref=external:owner/repo#issue/42")
    t.eq(fields[4], "attempt=2")
    t.eq(fields[5], "terminal=false")
  end,

  test_error_fact_fields_omit_unavailable_delivery_context = function()
    local fields = core.error_fact_fields("caught-failure", "github_poll_tick", "github_poll", "poll failed", {})

    t.eq(#fields, 2)
    t.eq(fields[1], "error_class=caught-failure")
    t.is_true(fields[2]:find("^fingerprint=fp%-") ~= nil)
  end,

  test_log_error_fact_emits_structured_failure_line = function()
    local captured = {}
    local old_log = log
    log = {
      warn = function(message)
        table.insert(captured, tostring(message))
      end,
    }

    core.log_error_fact("warn", "github_poll", "FAILURE", "gh-command-failed", "github_poll_tick", "gh failed", {
      source_ref = { kind = "external", ref = "owner/repo#issue/42" },
      terminal = false,
    })
    log = old_log

    t.eq(#captured, 1)
    t.is_true(captured[1]:find("github-proxy dept=github_poll tag=FAILURE", 1, true) ~= nil)
    t.is_true(captured[1]:find("error_class=gh-command-failed", 1, true) ~= nil)
    t.is_true(captured[1]:find("fingerprint=", 1, true) ~= nil)
    t.is_true(captured[1]:find("source_ref=external:owner/repo#issue/42", 1, true) ~= nil)
    t.is_true(captured[1]:find("terminal=false", 1, true) ~= nil)
  end,

  test_wrapped_pipeline_failure_logs_delivery_error_fact_and_rethrows = function()
    local captured = {}
    local old_log = log
    log = {
      error = function(message)
        table.insert(captured, tostring(message))
      end,
    }

    local wrapped = core.wrap_pipeline_failure("github_pr_open", function(_event)
      error("github-proxy: gh-pr-create-failed: bad sha abcdef1234567890 at 2026-06-10T01:02:03Z /tmp/fkst-a")
    end)
    local ok, err = pcall(function()
      wrapped({
        queue = "github_pr_open_request",
        attempt = 5,
        terminal = false,
        payload = {
          source_ref = { kind = "external", ref = "owner/repo#issue/42" },
        },
      })
    end)

    log = old_log
    t.eq(ok, false)
    t.is_true(tostring(err):find("gh-pr-create-failed", 1, true) ~= nil)
    t.eq(#captured, 1)
    t.is_true(captured[1]:find("github-proxy dept=github_pr_open tag=FAILURE", 1, true) ~= nil)
    t.is_true(captured[1]:find("error_class=gh-pr-create-failed", 1, true) ~= nil)
    t.is_true(captured[1]:find("fingerprint=", 1, true) ~= nil)
    t.is_true(captured[1]:find("source_ref=external:owner/repo#issue/42", 1, true) ~= nil)
    t.is_true(captured[1]:find("attempt=5", 1, true) ~= nil)
    t.is_nil(captured[1]:find("terminal=", 1, true))
    t.is_true(captured[1]:find("queue=github_pr_open_request", 1, true) ~= nil)
  end,

  test_gh_exec_fails_closed_for_non_rate_limit_failure = function()
    local ok, err = pcall(function()
      core.gh_exec("gh issue list", 30, "gh issue list", function(_spec)
        return { stdout = "", stderr = "GraphQL: field does not exist", exit_code = 1 }
      end)
    end)

    t.eq(ok, false)
    t.is_true(tostring(err):find("gh-command-failed", 1, true) ~= nil)
    t.eq(tostring(err):find("gh-rate-limited", 1, true), nil)
    t.eq(core.is_gh_rate_limit_error(err), false)
  end,

  test_gh_exec_raises_retryable_rate_limit_class = function()
    local ok, err = pcall(function()
      core.gh_exec("gh issue list", 30, "gh issue list", function(_spec)
        return { stdout = "", stderr = "API rate limit exceeded", exit_code = 1 }
      end)
    end)

    t.eq(ok, false)
    t.is_true(tostring(err):find("gh-rate-limited", 1, true) ~= nil)
    t.eq(core.is_gh_rate_limit_error(err), true)
  end,

  test_gh_commands_are_quoted = function()
    t.eq(
      core.gh_issue_list_cmd("owner/repo"),
      "gh api --paginate --slurp 'repos/owner/repo/issues?state=open&per_page=100'"
    )
    t.eq(
      core.gh_pr_list_cmd("owner/repo"),
      "gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&per_page=100'"
    )
    t.eq(
      core.gh_pr_list_head_cmd("owner/repo", "devloop-owner-repo-42-01HY"),
      "gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&head=owner%3Adevloop-owner-repo-42-01HY&per_page=100'"
    )
    t.eq(
      core.gh_pr_list_head_cmd("owner/repo", "devloop-owner-repo-42-01HY", "dev"),
      "gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&head=owner%3Adevloop-owner-repo-42-01HY&per_page=100&base=dev'"
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
    local rest_listed = core.parse_pr_list_for_head('[[{"number":8,"html_url":"https://example.test/8","head":{"ref":"devloop-owner-repo-42-01HY"},"base":{"ref":"dev"},"state":"open"}]]', "devloop-owner-repo-42-01HY")
    t.eq(rest_listed.number, 8)
    t.eq(rest_listed.url, "https://example.test/8")
    t.eq(rest_listed.base_ref_name, "dev")
    t.eq(core.parse_pr_list_for_head('[{"number":7,"headRefName":"devloop-owner-repo-42-01HY","state":"CLOSED"}]', "devloop-owner-repo-42-01HY"), nil)
    t.eq(
      core.gh_pr_view_head_oid_cmd("owner/repo", 7),
      "gh api 'repos/owner/repo/pulls/7'"
    )
    local same_repo_pr = core.parse_pr_view_head_state(
      '{"head":{"ref":"feature","sha":"ABC123","repo":{"full_name":"owner/repo","owner":{"login":"owner"}}},"base":{"ref":"dev","repo":{"full_name":"owner/repo","owner":{"login":"owner"}}},"state":"open","merged":false}',
      "owner/repo"
    )
    t.eq(same_repo_pr.head_ref_oid, "abc123")
    t.eq(same_repo_pr.state, "OPEN")
    t.eq(same_repo_pr.head_repository, "owner/repo")
    t.eq(same_repo_pr.is_target_repository, true)
    local empty_nwo_pr = core.parse_pr_view_head_state(
      '{"head":{"ref":"feature","sha":"ABC123","repo":{"name":"fkst-packages","owner":{"login":"ChronoAIProject"}}},"base":{"ref":"dev","repo":{"full_name":"ChronoAIProject/fkst-packages","owner":{"login":"ChronoAIProject"}}},"state":"closed","merged":true}',
      "ChronoAIProject/fkst-packages"
    )
    t.eq(empty_nwo_pr.head_repository, "ChronoAIProject/fkst-packages")
    t.eq(empty_nwo_pr.is_target_repository, true)
    t.eq(core.parse_pr_view_head_state(
      '{"head":{"ref":"feature","sha":"ABC123","repo":{"full_name":"fork/repo","owner":{"login":"fork"}}},"base":{"ref":"dev","repo":{"full_name":"owner/repo","owner":{"login":"owner"}}},"state":"open","merged":false}',
      "owner/repo"
    ).is_target_repository, false)
    t.eq(core.parse_pr_create("https://example.test/pull/8\n").number, 8)
    t.eq(
      core.gh_issue_view_comments_cmd("owner/repo", 3),
      "gh api --paginate --slurp 'repos/owner/repo/issues/3/comments?per_page=100'"
    )
    local expected_label_colors = {
      ["fkst-dev:enabled"] = "1D76DB",
      ["fkst-dev:tracking"] = "C5DEF5",
      ["fkst-dev:thinking"] = "8250DF",
      ["fkst-dev:ready"] = "0E8A16",
      ["fkst-dev:implementing"] = "FBCA04",
      ["fkst-dev:pr-open"] = "006B75",
      ["fkst-dev:reviewing"] = "5319E7",
      ["fkst-dev:fixing"] = "D93F0B",
      ["fkst-dev:merge-ready"] = "2EA44F",
      ["fkst-dev:merging"] = "C2E0C6",
      ["fkst-dev:merged"] = "8957E5",
      ["fkst-dev:impl-failed"] = "B60205",
      ["fkst-dev:blocked"] = "1B1F23",
      ["fkst-dev:blocked-on-dependency"] = "E99695",
      ["fkst-dev:review-meta"] = "BFD4F2",
    }
    for label, color in pairs(expected_label_colors) do
      t.eq(
        core.gh_label_create_cmd("owner/repo", label),
        "gh label create '" .. label .. "' --repo 'owner/repo' --color '" .. color .. "'"
      )
    end
    t.eq(
      core.gh_label_create_cmd("owner/repo", "custom'label"),
      "gh label create 'custom'\\''label' --repo 'owner/repo' --color 'ededed'"
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
