-- std module behavior tests are hosted in github-proxy (a flat package, the
-- strictest single-root conformance gate) because the engine test runner only
-- scans <root>/tests and <root>/departments/* (no recursion into std/tests).
local strings = require("std.strings")
local t = fkst.test

return {
  test_trim_strips_both_ends = function()
    t.eq(strings.trim("  hi  "), "hi")
    t.eq(strings.trim(nil), "")
  end,

  test_split_repo_accepts_single_owner_name_separator = function()
    local owner, name = strings.split_repo("owner/repo")
    t.eq(owner, "owner")
    t.eq(name, "repo")
  end,

  test_split_repo_rejects_missing_or_extra_separator = function()
    local owner, name = strings.split_repo("owner")
    t.is_nil(owner)
    t.is_nil(name)

    owner, name = strings.split_repo("owner/repo/extra")
    t.is_nil(owner)
    t.is_nil(name)

    owner, name = strings.split_repo(nil)
    t.is_nil(owner)
    t.is_nil(name)
  end,

  test_comment_body_normalizes_table_string_and_nil = function()
    t.eq(strings.comment_body({ body = "hello" }), "hello")
    t.eq(strings.comment_body({ body = nil }), "")
    t.eq(strings.comment_body("plain"), "plain")
    t.eq(strings.comment_body(nil), "")
  end,

  test_json_string_wraps_and_escapes_json_string_boundaries = function()
    t.eq(strings.json_string(nil), '""')
    t.eq(strings.json_string('a"b\\c'), '"a\\"b\\\\c"')
  end,

  test_json_string_escapes_c0_control_characters = function()
    t.eq(strings.json_string("\b\f\n\r\t"), '"\\b\\f\\n\\r\\t"')
    t.eq(strings.json_string("x" .. string.char(0) .. string.char(31) .. "y"), '"x\\u0000\\u001fy"')
  end,

  test_json_string_documents_temporary_canonical_encoder_waiver = function()
    local source = file.read("std/strings.lua")
    t.is_true(source:find("std.strings.json_string is a temporary byte-identical stopgap", 1, true) ~= nil)
    t.is_true(source:find("canonical JSON encoding remains deferred to a dedicated encoder boundary", 1, true) ~= nil)
  end,

  test_bounded_string_requires_non_empty_string_under_limit = function()
    t.is_true(strings.is_bounded_string("abc", 3))
    t.eq(strings.is_bounded_string("abcd", 3), false)
    t.eq(strings.is_bounded_string("", 3), false)
    t.eq(strings.is_bounded_string(123, 3), false)
  end,

  test_git_ref_safe_rejects_unsafe_boundaries_and_accepts_normal_ref = function()
    t.is_true(strings.is_git_ref_safe("feat/x"))
    t.eq(strings.is_git_ref_safe("-feat/x"), false)
    t.eq(strings.is_git_ref_safe("/feat/x"), false)
    t.eq(strings.is_git_ref_safe("feat/../x"), false)
    t.eq(strings.is_git_ref_safe("feat//x"), false)
    t.eq(strings.is_git_ref_safe("feat/@{x"), false)
    t.eq(strings.is_git_ref_safe("feat/x/"), false)
    t.eq(strings.is_git_ref_safe("feat/x."), false)
    t.eq(strings.is_git_ref_safe(("a"):rep(161)), false)
  end,

  test_decimal_checksum_matches_existing_package_algorithm = function()
    t.eq(strings.decimal_checksum("DRY/Rule-of-Three"), "1383444728")
  end,

  test_path_safe_key_rejects_absolute_backslash_space_and_dot_segments = function()
    t.is_true(strings.is_path_safe_key("owner/repo#issue/42", 200))
    t.is_true(strings.is_path_safe_key("cache_key.v1-2/part", 200))
    t.eq(strings.is_path_safe_key("/owner/repo", 200), false)
    t.eq(strings.is_path_safe_key("owner\\repo", 200), false)
    t.eq(strings.is_path_safe_key("owner repo", 200), false)
    t.eq(strings.is_path_safe_key("owner/../repo", 200), false)
    t.eq(strings.is_path_safe_key("owner/<repo>", 200), false)
    t.eq(strings.is_path_safe_key(("a"):rep(201), 200), false)
  end,

  test_sanitize_key_preserves_path_chars_and_clamps_segments = function()
    t.eq(strings.sanitize_key(" owner/repo#issue 42 "), "-owner/repo#issue-42-")
    t.eq(strings.sanitize_key("/owner//./../repo/"), "owner/-/-/repo")
    t.eq(strings.sanitize_key(nil), "empty")
    t.eq(strings.sanitize_key("abc/def", 5), "abc/d")
    t.eq(strings.sanitize_key("abc/def", false), "abc/def")
  end,
}
