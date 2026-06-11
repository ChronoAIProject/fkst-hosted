local h = require("tests.devloop_core_helpers")
local fixtures = require("tests.production_fixture_helpers")
require("tests.context_bundle_probe_helpers")
local core = h.core
local t = h.t

local function nonce()
  return tostring({}):gsub("[^%w._-]", "_")
end

local function runtime_root(name)
  return "/tmp/fkst-packages-test/github-devloop-context-bundle/" .. tostring(now()) .. "/" .. nonce() .. "/" .. name
end

local function run_probe(mode, root)
  local result = t.run_department("tests/context_bundle_probe_helpers.lua", {
    queue = "context_bundle_probe",
    payload = {
      mode = mode,
      root = root,
    },
  }, {
    env = {
      FKST_RUNTIME_ROOT = root,
    },
  })
  t.eq(result.exit_code, 0)
  for _, raised in ipairs(result.raises or {}) do
    if raised.queue == "context_bundle_probe_result" then
      return raised.payload
    end
  end
  error("missing context bundle probe result")
end

local function assert_valid_utf8(value)
  local ok, len = pcall(utf8.len, tostring(value or ""))
  t.is_true(ok and len ~= nil)
end

local function assert_consensus_safe_context_key(key)
  t.is_true(#key <= 180)
  t.is_true(#key <= 200)
  t.is_true(core._is_path_safe_key(key, 200))
  t.is_true(key:sub(1, 1) ~= "/")
  t.is_nil(key:find("\\", 1, true))
  t.is_nil(key:find("%s"))
end

return {
  test_context_bundle_cache_keys_bound_realistic_pr_review_proposal_id = function()
    local proposal_id = "github-devloop/pr-review/ChronoAIProject/fkst-packages/2376452037/223/ready-consensus-github-devloop-issue-ChronoAIProject-fkst-packages-221-2026-06-10T20-13-08Z-2548858339"
    local version = proposal_id .. "/review/loop/17/review-meta/2026-06-10T21-14-55Z-9988776655"
    local bundle_key = core.context_bundle_key(proposal_id, version)
    local manifest_key = core.context_bundle_manifest_key(proposal_id, version)

    assert_consensus_safe_context_key(bundle_key)
    assert_consensus_safe_context_key(manifest_key)
  end,

  test_context_bundle_cache_keys_keep_long_proposal_ids_distinct = function()
    local proposal_a = "github-devloop/pr-review/ChronoAIProject/fkst-packages/2376452037/223/ready-consensus-github-devloop-issue-ChronoAIProject-fkst-packages-221-2026-06-10T20-13-08Z-2548858339"
    local proposal_b = "github-devloop/pr-review/ChronoAIProject/fkst-packages/2376452037/223/ready-consensus-github-devloop-issue-ChronoAIProject-fkst-packages-221-2026-06-10T20-13-08Z-0000000000"
    local version = "review-loop-2026-06-10T21-14-55Z"

    t.is_true(core.context_bundle_key(proposal_a, version) ~= core.context_bundle_key(proposal_b, version))
    t.is_true(core.context_bundle_manifest_key(proposal_a, version) ~= core.context_bundle_manifest_key(proposal_b, version))
  end,

  test_context_bundle_cache_keys_keep_short_id_behavior = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = "v1"

    t.eq(core.context_bundle_key(proposal_id, version), "github-devloop/context-bundle/github-devloop/issue/owner/repo/42/v1")
    t.eq(core.context_bundle_manifest_key(proposal_id, version), "github-devloop/context-bundle-manifest/github-devloop/issue/owner/repo/42/v1")
  end,

  test_context_bundle_files_round_trip_from_different_cwd = function()
    local result = run_probe("round_trip", runtime_root("round-trip"))

    t.eq(#result.paths, 5)
    t.is_true(result.manifest:find("UNTRUSTED-NOTICE.txt", 1, true) ~= nil)
    t.is_true(result.manifest:find("bytes):", 1, true) ~= nil)
    t.is_true(result.manifest:find("Files may be large; read them in segments as needed.", 1, true) ~= nil)
    t.is_true(result.notice_content:find("BEGIN UNTRUSTED BUNDLE DATA", 1, true) == 1)
    t.is_true(result.issue_content:find("{", 1, true) == 1)
    t.is_nil(result.issue_content:find(core._untrusted_issue_data_begin, 1, true))
  end,

  test_context_bundle_cache_hit_with_deleted_file_rebuilds = function()
    local result = run_probe("deleted_file", runtime_root("deleted-file"))

    t.is_true(result.second_dir ~= result.first_dir)
    t.is_true(result.second_dir:find(result.first_dir .. ".publish-", 1, true) == 1)
    t.is_true(result.issue_content:find("Second issue", 1, true) ~= nil)
    t.eq(result.issue_fetch_count, 2)
  end,

  test_context_bundle_reuses_preexisting_final_dir_after_validation = function()
    local result = run_probe("preexisting", runtime_root("preexisting-final"))

    t.eq(result.dir, result.expected_dir)
    t.is_true(result.issue_content:find("preexisting issue", 1, true) ~= nil)
    t.is_true(result.manifest:find("UNTRUSTED-NOTICE.txt", 1, true) ~= nil)
    t.eq(result.issue_fetch_count, 0)
  end,

  test_context_bundle_second_publish_reuses_valid_final_dir = function()
    local result = run_probe("publish_reuse", runtime_root("publish-reuse"))

    t.eq(result.second_dir, result.first_dir)
    t.eq(result.fetches_after_first, 1)
    t.eq(result.fetches_after_second, 1)
    t.eq(result.notice_unchanged, true)
    t.eq(result.issue_unchanged, true)
    t.eq(result.board_unchanged, true)
  end,

  test_context_bundle_second_publish_uses_unique_dir_when_final_invalid = function()
    local result = run_probe("publish_unique_on_invalid", runtime_root("publish-unique-invalid"))

    t.is_true(result.dir ~= result.original_dir)
    t.is_true(result.dir:find(result.original_dir .. ".publish-", 1, true) == 1)
    t.eq(result.issue_fetch_count, 2)
    t.eq(result.original_notice_absent, true)
    t.eq(result.original_issue_unchanged, true)
    t.eq(result.original_board_unchanged, true)
    t.is_true(result.rebuilt_issue:find("Rebuilt issue", 1, true) ~= nil)
    t.eq(result.has_notice, true)
    t.is_true(result.manifest:find("UNTRUSTED-NOTICE.txt", 1, true) ~= nil)
  end,

  test_context_bundle_file_cap_truncates_on_utf8_boundary = function()
    local result = run_probe("utf8_truncation", runtime_root("utf8-truncation"))

    t.eq(result.issue_bytes, core._max_bundle_file_len - 1)
    assert_valid_utf8(result.issue_content)
  end,

  test_context_bundle_manifest_key_accepts_full_pr_review_proposal_id = function()
    local repo = fixtures.long_repo()
    local version = fixtures.full_review_issue_version(repo)
    local proposal_id = core.pr_review_proposal_id(repo, 187, version, fixtures.review_head_sha())
    local manifest_key = core.context_bundle_manifest_key(proposal_id, version)
    local bundle_key = core.context_bundle_key(proposal_id, version)

    t.is_true(#fixtures.unbounded_full_review_proposal_id() > core._max_key_len)
    t.is_true(#proposal_id <= core._max_key_len)
    t.is_true(core.context_bundle_key("github-devloop/issue/owner/repo/42", "owner/repo#issue#42@2026-06-03T01:02:03Z"):find("#", 1, true) == nil)
    t.is_true(#manifest_key <= core._max_key_len)
    t.is_true(#bundle_key <= core._max_key_len)
    t.eq(core._is_path_safe_key(manifest_key, core._max_key_len), true)
    t.eq(core._is_path_safe_key(bundle_key, core._max_key_len), true)
    t.is_true(manifest_key:find("^github%-devloop/context%-bundle%-manifest/") ~= nil)
    t.is_true(bundle_key:find("^github%-devloop/context%-bundle/") ~= nil)
  end,
}
