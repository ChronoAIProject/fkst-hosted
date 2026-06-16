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

  test_bounded_string_requires_non_empty_string_under_limit = function()
    t.is_true(strings.is_bounded_string("abc", 3))
    t.eq(strings.is_bounded_string("abcd", 3), false)
    t.eq(strings.is_bounded_string("", 3), false)
    t.eq(strings.is_bounded_string(123, 3), false)
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
