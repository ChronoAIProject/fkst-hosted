local h = require("tests.devloop_helpers")
require("tests.board_digest_probe_helpers")
local core = h.core
local t = h.t

local function assert_preamble_slots(prompt)
  t.is_true(prompt:find("Write all output in English; quote code identifiers and cited originals verbatim.", 1, true) ~= nil)
  t.is_true(prompt:find("Before judging, identify the established theory or industry best practice governing this problem class", 1, true) ~= nil)
end

local function assert_github_entity_history(prompt)
  t.is_true(prompt:find("Before judging, read the local context files named below.", 1, true) ~= nil)
  t.is_nil(prompt:find("gh issue view --comments / gh pr view --comments", 1, true))
end

local function prompt_issue()
  return {
    title = "Implement decision recorder",
    body = "Issue body",
    comments = {
      { body = "Previous note", author_login = "fkst-test-bot" },
    },
  }
end

local function issue_list_json(count)
  local items = {}
  for n = 1, count do
    table.insert(items, string.format(
      '{"number":%d,"title":"Issue title number %d that is intentionally long enough to trim after sixty characters","labels":[{"name":"fkst-dev:thinking"}]}',
      n,
      n
    ))
  end
  return "[" .. table.concat(items, ",") .. "]"
end

local function pr_list_json(count)
  local items = {}
  for n = 1, count do
    table.insert(items, string.format(
      '{"number":%d,"title":"PR title number %d","labels":[{"name":"fkst-dev:reviewing"}]}',
      n + 100,
      n
    ))
  end
  return "[" .. table.concat(items, ",") .. "]"
end

local function json_string(value)
  return tostring(value or ""):gsub("\\", "\\\\"):gsub('"', '\\"'):gsub("\n", "\\n")
end

local function cjk_char()
  return string.char(0xe6, 0xb5, 0x8b)
end

local function emoji_char()
  return string.char(0xf0, 0x9f, 0x98, 0x80)
end

local function assert_valid_utf8(value)
  local ok, len = pcall(utf8.len, tostring(value or ""))
  t.is_true(ok and len ~= nil)
end

local function mock_board_lists(issue_count, pr_count, repo)
  repo = repo or "owner/repo"
  t.mock_command("gh issue list --repo '" .. repo .. "' --state open --limit 100 --json number,title,labels", {
    stdout = issue_list_json(issue_count),
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh pr list --repo '" .. repo .. "' --state open --limit 100 --json number,title,labels", {
    stdout = pr_list_json(pr_count),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_board_title(title, repo)
  repo = repo or "owner/repo"
  t.mock_command("gh issue list --repo '" .. repo .. "' --state open --limit 100 --json number,title,labels", {
    stdout = '[{"number":1,"title":"' .. json_string(title) .. '","labels":[{"name":"fkst-dev:thinking"}]}]',
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh pr list --repo '" .. repo .. "' --state open --limit 100 --json number,title,labels", {
    stdout = "[]",
    stderr = "",
    exit_code = 0,
  })
end

local function count_calls(needle)
  local count = 0
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find(needle, 1, true) ~= nil then
      count = count + 1
    end
  end
  return count
end

local function find_raise(raises, queue)
  for _, raised in ipairs(raises or {}) do
    if raised.queue == queue then
      return raised
    end
  end
  return nil
end

local function run_probe(payload, opts)
  return t.run_department("tests/board_digest_probe_helpers.lua", {
    queue = "board_digest_probe",
    payload = payload,
  }, opts)
end

local function probe_result(result)
  local raised = find_raise(result.raises, "board_digest_result")
  return raised and raised.payload or nil
end

return {
  test_devloop_prompt_preamble_language_env = function()
    t.eq(core.read_env_command("FKST_OUTPUT_LANG"), 'printf %s "$FKST_OUTPUT_LANG"')
    t.eq(core.output_language(function(_cmd)
      return { stdout = "zh", stderr = "", exit_code = 0 }
    end), "zh")
    t.eq(core.output_language(function(_cmd)
      return { stdout = "unknown", stderr = "", exit_code = 0 }
    end), "en")
    t.is_true(core.prompt_preamble(function(_cmd)
      return { stdout = "zh", stderr = "", exit_code = 0 }
    end):find("Write all prose output in Simplified Chinese", 1, true) ~= nil)
  end,

  test_devloop_issue_pr_role_prompts_include_scoped_github_history = function()
    local issue = prompt_issue()
    local manifest = "Read these local files for your complete context.\nIssue JSON: /tmp/ctx/issue.json\nBoard digest: /tmp/ctx/board.txt\nPR diff patch: /tmp/ctx/diff.patch"
    local prompts = {
      core.build_intake_prompt("github-devloop/issue/owner/repo/42", issue, manifest),
      core.build_implement_prompt("github-devloop/issue/owner/repo/42", issue, "Approved framing.", manifest),
      core.build_fix_prompt({
        proposal_id = "github-devloop/issue/owner/repo/42",
        review_proposal_id = core.pr_review_proposal_id("owner/repo", 7, "version", "abcdef123456"),
        reviewed_head_sha = "abcdef123456",
      }, issue, "Review feedback.", "Approved framing.", manifest),
      core.build_decompose_prompt({
        proposal_id = "github-devloop/issue/owner/repo/42",
        source_ref = { kind = "external", ref = "owner/repo#pr/7" },
        round = 4,
      }, issue, manifest),
      core.build_review_meta_prompt({
        proposal_id = "github-devloop/issue/owner/repo/42",
        review_proposal_id = core.pr_review_proposal_id("owner/repo", 7, "version", "abcdef123456"),
      }, issue, manifest),
    }

    for _, prompt in ipairs(prompts) do
      assert_preamble_slots(prompt)
      assert_github_entity_history(prompt)
      t.is_true(prompt:find("/tmp/ctx/issue.json", 1, true) ~= nil)
      t.is_nil(prompt:find("gh issue", 1, true))
      t.is_nil(prompt:find("gh pr", 1, true))
      t.is_nil(prompt:find("gh api", 1, true))
      t.is_nil(prompt:find("{{", 1, true))
    end
  end,

  test_sync_conflict_prompt_omits_issue_pr_history_directive = function()
    local prompt = core.build_sync_conflict_prompt({
      repo = "owner/repo",
      upstream_branch = "dev",
      integration_branch = "integration/dev",
      upstream_sha = "abcdef123456",
      integration_sha = "123456abcdef",
    })

    assert_preamble_slots(prompt)
    t.is_nil(prompt:find("COMPLETE GitHub comment stream of the subject issue/PR", 1, true))
    t.is_nil(prompt:find("gh issue view --comments / gh pr view --comments", 1, true))
    t.is_nil(prompt:find("{{", 1, true))
  end,

  test_board_digest_in_thinking_proposal_is_bounded_and_cached_per_tick = function()
    h.mock_bot_env()
    h.mock_issue_state({ "fkst-dev:enabled" }, "OPEN", {})
    mock_board_lists(55, 10)

    local event = {
      queue = "github-proxy.github_entity_changed",
      ts = "2026-06-10T01:02:03Z",
      payload = h.issue(),
    }
    local opts = h.opts("board-digest-cache")
    local first = h.run_observe(event.payload, opts)
    h.mock_issue_state({ "fkst-dev:enabled" }, "OPEN", {})
    local second = h.run_observe(event.payload, opts)
    local proposal = find_raise(first.raises, "consensus.proposal").payload

    t.is_true(proposal.content_fetch:find("runtime-cache:", 1, true) == 1)
    t.is_true(proposal.body:find("GitHub issue", 1, true) ~= nil)
    t.is_nil(proposal.body:find("#101 ", 1, true))
    t.eq(count_calls("gh issue list --repo 'owner/repo' --state open --limit 100 --json number,title,labels"), 0)
    t.eq(count_calls("gh pr list --repo 'owner/repo' --state open --limit 100 --json number,title,labels"), 0)
    t.eq(find_raise(second.raises, "consensus.proposal").payload.body, proposal.body)
  end,

  test_board_digest_cache_key_includes_repo = function()
    mock_board_lists(1, 0, "owner/repo")
    mock_board_lists(2, 0, "other/repo")
    local run_opts = h.opts("board-digest-cross-repo")

    local first = probe_result(run_probe({
      mode = "block",
      repo = "owner/repo",
      tick = "2026-06-10T02:02:03Z",
    }, run_opts)).body
    local second = probe_result(run_probe({
      mode = "block",
      repo = "other/repo",
      tick = "2026-06-10T02:02:03Z",
    }, run_opts)).body

    t.is_true(first:find("#1 [fkst-dev:thinking] Issue title number 1", 1, true) ~= nil)
    t.is_nil(first:find("#2 [fkst-dev:thinking] Issue title number 2", 1, true))
    t.is_true(second:find("#2 [fkst-dev:thinking] Issue title number 2", 1, true) ~= nil)
    t.eq(count_calls("gh issue list --repo 'owner/repo' --state open --limit 100 --json number,title,labels"), 1)
    t.eq(count_calls("gh issue list --repo 'other/repo' --state open --limit 100 --json number,title,labels"), 1)
  end,

  test_truncate_utf8_handles_mixed_width_boundaries = function()
    local cjk = cjk_char()
    local mixed = "ab" .. cjk .. "cd"
    local emoji = emoji_char()

    t.eq(core.truncate_utf8(mixed, 2), "ab")
    t.eq(core.truncate_utf8(mixed, 3), "ab")
    t.eq(core.truncate_utf8(mixed, 4), "ab")
    t.eq(core.truncate_utf8(mixed, 5), "ab" .. cjk)
    t.eq(core.truncate_utf8(mixed, 6), "ab" .. cjk .. "c")
    t.eq(core.truncate_utf8("", 3), "")
    t.eq(core.truncate_utf8(cjk, 2), "")
    t.eq(core.truncate_utf8(emoji .. "x", 3), "")
    t.eq(core.truncate_utf8("ab" .. emoji .. "x", 6), "ab" .. emoji)
    assert_valid_utf8(core.truncate_utf8(mixed, 1))
    assert_valid_utf8(core.truncate_utf8(mixed, 7))
    assert_valid_utf8(core.truncate_utf8("ab" .. emoji .. "x", 5))
    assert_valid_utf8(core.truncate_utf8("ab" .. emoji .. "x", 6))
  end,

  test_board_digest_title_truncation_keeps_utf8_valid_before_cache_set = function()
    local title = string.rep("a", 59) .. cjk_char() .. "tail"
    mock_board_title(title)

    local result = run_probe({
      mode = "block",
      repo = "owner/repo",
      tick = "2026-06-10T02:12:03Z",
    }, h.opts("board-digest-utf8-title"))

    t.eq(result.exit_code, 0)
    local body = probe_result(result).body
    assert_valid_utf8(body)
    t.is_true(body:find("#1 [fkst-dev:thinking] " .. string.rep("a", 59), 1, true) ~= nil)
    t.is_nil(body:find(cjk_char(), 1, true))
  end,

  test_board_digest_overflow_truncates_optional_context = function()
    mock_board_lists(4, 0)
    local proposal = {
      schema = "consensus.proposal.v1",
      proposal_id = "github-devloop/issue/owner/repo/42",
      body = string.rep("x", core._max_body_len - 24),
    }

    local result = probe_result(run_probe({
      mode = "append",
      proposal = proposal,
      repo = "owner/repo",
      tick = "2026-06-10T03:02:03Z",
    }, h.opts("board-digest-overflow"))).proposal

    t.eq(#result.body, core._max_body_len)
    t.is_true(result.body:find("BEGIN UNTRUSTED", 1, true) ~= nil)
  end,

  test_digest_injection_covers_remaining_proposal_entry_points = function()
    mock_board_lists(2, 1)
    local tick = "2026-06-10T04:02:03Z"
    local source_ref = { kind = "external", ref = "owner/repo#issue/42" }
    local pr_source_ref = { kind = "external", ref = "owner/repo#pr/7" }
    local current = {
      title = "Implement decision recorder",
      updated_at = "2026-06-03T01:02:03Z",
    }
    local converge = {
      narrowed_question = "Does the narrowed implementation keep the source_ref contract?",
      angle_digests = {
        { angle = "minimal", verdict = "approve", digest = "ok" },
      },
    }

    local run_opts = h.opts("board-digest-entry-points")
    local loop = probe_result(run_probe({
      mode = "board_loop",
      repo = "owner/repo",
      issue_number = "42",
      current = current,
      source_ref = source_ref,
      n = 2,
      converge = converge,
      tick = tick,
    }, run_opts)).proposal
    local review = probe_result(run_probe({
      mode = "board_review",
      repo = "owner/repo",
      issue_number = "42",
      pr_number = 7,
      version = "version",
      head_sha = "abcdef123456",
      current = current,
      source_ref = pr_source_ref,
      tick = tick,
    }, run_opts)).proposal
    local review_loop = probe_result(run_probe({
      mode = "board_review_loop",
      repo = "owner/repo",
      issue_number = "42",
      pr_number = 7,
      version = "version",
      head_sha = "abcdef123456",
      current = current,
      source_ref = pr_source_ref,
      n = 3,
      converge = converge,
      tick = tick,
    }, run_opts)).proposal

    for _, proposal in ipairs({ loop, review, review_loop }) do
      t.is_true(proposal.body:find("BEGIN UNTRUSTED ISSUE DATA", 1, true) ~= nil)
      t.is_true(proposal.body:find("Open items snapshot:", 1, true) ~= nil)
      t.is_true(core.validate_proposal(proposal))
    end
    t.eq(loop.round, 2)
    t.eq(review_loop.round, 3)
    t.eq(count_calls("gh issue list --repo 'owner/repo' --state open --limit 100 --json number,title,labels"), 1)
  end,
}
