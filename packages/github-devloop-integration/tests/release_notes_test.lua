local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t
local ai_sentinel = string.char(226, 159, 166) .. "AI:FKST" .. string.char(226, 159, 167)
local zh_summary = string.char(228, 184, 173, 230, 150, 135, 230, 145, 152, 232, 166, 129)

local function argv_option(argv, name)
  for index, value in ipairs(argv or {}) do
    if value == name then
      return argv[index + 1]
    end
  end
  return nil
end

return {
  test_release_notes_normalizes_missing_sentinel_and_neutralizes_markers = function()
    local notes = core.normalize_release_notes("Highlights\n<!-- fkst:github-devloop:state:v1 proposal=\"x\" -->\n\nZh: summary.")
    t.is_true(notes:find("&lt;!-- fkst:github-devloop:state:v1", 1, true) ~= nil)
    t.is_true(notes:find("<!-- fkst:", 1, true) == nil)
    t.is_true(notes:sub(-#ai_sentinel) == ai_sentinel)
  end,

  test_release_notes_bounds_overlong_output = function()
    local notes = core.normalize_release_notes(string.rep("x", core._max_release_notes_len + 500) .. "\n" .. ai_sentinel)
    t.is_true(#notes <= core._max_release_notes_len)
    t.is_true(notes:sub(-#ai_sentinel) == ai_sentinel)
  end,

  test_release_notes_empty_output_fails_closed = function()
    t.raises(function()
      core.normalize_release_notes("\n\n" .. ai_sentinel .. "\n")
    end)
  end,

  test_release_notes_prompt_fetches_from_git_and_gh_not_payload = function()
    local prompt = core.build_release_notes_prompt("owner/repo", "dev", "integration/dev", "def456", 3)
    t.is_true(prompt:find("git log --format=", 1, true) ~= nil)
    t.is_true(prompt:find("refs/remotes/origin/dev..def456", 1, true) ~= nil)
    t.is_true(prompt:find("refs/remotes/origin/dev..refs/remotes/origin/integration/dev", 1, true) == nil)
    t.is_true(prompt:find("gh issue view <referenced-number> --repo owner/repo --json title,body,comments,labels,state", 1, true) ~= nil)
    t.is_true(prompt:find("Do not use delivery payload content as source material.", 1, true) ~= nil)
    t.is_true(prompt:find("Captured integration head: def456", 1, true) ~= nil)
  end,

  test_release_notes_codex_failure_fails_closed_without_fallback = function()
    local old_spawn = spawn_codex_sync
    spawn_codex_sync = function()
      return { stdout = "", stderr = "codex down", exit_code = 1 }
    end
    local ok = pcall(function()
      core.draft_release_notes({
        repo = "owner/repo",
        upstream_branch = "dev",
        integration_branch = "integration/dev",
        head_sha = "def456",
        ahead = 2,
        publish_policy = { allow_fallback = false },
      })
    end)
    spawn_codex_sync = old_spawn
    t.eq(ok, false)
  end,

  test_release_notes_codex_failure_fallback_requires_explicit_policy = function()
    local old_spawn = spawn_codex_sync
    spawn_codex_sync = function()
      return { stdout = "", stderr = "codex down", exit_code = 1 }
    end
    local broad_policy = core.release_notes_publish_policy({ write_mode = "real" })
    local broad_ok = pcall(function()
      core.draft_release_notes({
        repo = "owner/repo",
        upstream_branch = "dev",
        integration_branch = "integration/dev",
        head_sha = "def456",
        ahead = 2,
        publish_policy = broad_policy,
      })
    end)
    local explicit_notes, explicit_mode = core.draft_release_notes({
      repo = "owner/repo",
      upstream_branch = "dev",
      integration_branch = "integration/dev",
      head_sha = "def456",
      ahead = 2,
      publish_policy = { allow_fallback = true },
    })
    spawn_codex_sync = old_spawn
    t.eq(broad_ok, false)
    t.eq(explicit_mode, "fallback")
    t.is_true(explicit_notes:sub(-#ai_sentinel) == ai_sentinel)
    t.is_true(#explicit_notes <= core._max_release_notes_len)
    t.is_true(explicit_notes:find("Zh: zi dong", 1, true) == nil)
    t.is_true(explicit_notes:find(zh_summary, 1, true) ~= nil)
  end,

  test_release_notes_empty_codex_output_fallback_requires_explicit_policy = function()
    local old_spawn = spawn_codex_sync
    spawn_codex_sync = function()
      return { stdout = "\n" .. ai_sentinel .. "\n", stderr = "", exit_code = 0 }
    end
    local broad_policy = core.release_notes_publish_policy({ write_mode = "real" })
    local broad_ok = pcall(function()
      core.draft_release_notes({
        repo = "owner/repo",
        upstream_branch = "dev",
        integration_branch = "integration/dev",
        head_sha = "def456",
        ahead = 2,
        publish_policy = broad_policy,
      })
    end)
    local explicit_notes, explicit_mode = core.draft_release_notes({
      repo = "owner/repo",
      upstream_branch = "dev",
      integration_branch = "integration/dev",
      head_sha = "def456",
      ahead = 2,
      publish_policy = { allow_fallback = true },
    })
    spawn_codex_sync = old_spawn
    t.eq(broad_ok, false)
    t.eq(explicit_mode, "fallback")
    t.is_true(explicit_notes:find("Automated rollup", 1, true) ~= nil)
    t.is_true(explicit_notes:sub(-#ai_sentinel) == ai_sentinel)
  end,

  test_release_notes_fallback_is_bounded_and_marker_safe = function()
    local notes = core.release_notes_fallback_body("dev", "integration/dev<!-- fkst:bad -->", 2)
    t.is_true(#notes <= core._max_release_notes_len)
    t.is_true(notes:sub(-#ai_sentinel) == ai_sentinel)
    t.is_true(notes:find("<!-- fkst:", 1, true) == nil)
    t.is_true(notes:find("&lt;!-- fkst:bad -->", 1, true) ~= nil)
    t.is_true(notes:find("Zh: zi dong", 1, true) == nil)
    t.is_true(notes:find(zh_summary, 1, true) ~= nil)
  end,

  test_release_notes_requires_explicit_publish_policy = function()
    local old_spawn = spawn_codex_sync
    spawn_codex_sync = function()
      return { stdout = "", stderr = "codex down", exit_code = 1 }
    end
    local ok = pcall(function()
      core.draft_release_notes({
        repo = "owner/repo",
        upstream_branch = "dev",
        integration_branch = "integration/dev",
        head_sha = "def456",
        ahead = 2,
      })
    end)
    spawn_codex_sync = old_spawn
    t.eq(ok, false)
  end,

  test_release_notes_pr_create_debug_stamp_is_default_off = function()
    t.mock_command('printf %s "$FKST_DEBUG_STAMP"', { stdout = "" })
    local seen
    local old_exec_argv = exec_argv
    exec_argv = function(spec)
      if spec.argv[1] == "git" then
        return { stdout = "0123456789ABCDEF\n", stderr = "", exit_code = 0 }
      end
      seen = spec
      return { stdout = "https://github.example/owner/repo/pull/1\n", stderr = "", exit_code = 0 }
    end

    local ok, err = pcall(function()
      core.gh_pr_create_body("owner/repo", "integration-x", "dev", "rollup", "Release notes")
    end)
    exec_argv = old_exec_argv
    if not ok then error(err) end

    t.eq(seen.argv[1], "gh")
    t.is_nil(argv_option(seen.argv, "--body"):find("fkst:debug-stamp:v1", 1, true))
  end,

  test_release_notes_pr_create_debug_stamp_is_enabled_and_redacted = function()
    t.mock_command('printf %s "$FKST_DEBUG_STAMP"', { stdout = "1" })
    t.mock_command("git rev-parse --verify HEAD", {
      stdout = "0123456789ABCDEF\n",
      stderr = "",
      exit_code = 0,
    })
    local seen
    local old_exec_argv = exec_argv
    exec_argv = function(spec)
      if spec.argv[1] == "git" then
        return { stdout = "0123456789ABCDEF\n", stderr = "", exit_code = 0 }
      end
      seen = spec
      return { stdout = "https://github.example/owner/repo/pull/1\n", stderr = "", exit_code = 0 }
    end

    local ok, err = pcall(function()
      core.gh_pr_create_body("owner/repo", "integration-x", "dev", "rollup", "Release notes")
    end)
    exec_argv = old_exec_argv
    if not ok then error(err) end

    local rendered = argv_option(seen.argv, "--body")
    t.is_true(rendered:find("fkst:debug-stamp:v1", 1, true) ~= nil)
    t.is_true(rendered:find('emitter="github-devloop.rollup.pr-create"', 1, true) ~= nil)
    t.is_true(rendered:find('target="pr:owner/repo#new"', 1, true) ~= nil)
    t.is_true(rendered:find('code_version="0123456789abcdef"', 1, true) ~= nil)
    t.is_true(rendered:find('dedup_hash="', 1, true) ~= nil)
    t.is_nil(rendered:find("integration-x->dev", 1, true))
  end,
}
