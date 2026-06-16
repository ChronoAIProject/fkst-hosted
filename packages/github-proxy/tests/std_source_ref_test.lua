-- std module behavior tests are hosted in github-proxy (a flat package, the
-- strictest single-root conformance gate) because the engine test runner only
-- scans <root>/tests and <root>/departments/* (no recursion into std/tests).
local source_ref = require("std.source_ref")
local t = fkst.test

return {
  test_bounded_source_ref_requires_kind_and_ref_under_limit = function()
    t.is_true(source_ref.has_bounded_source_ref({
      kind = "external",
      ref = "owner/repo#issue/42",
    }, 200))
    t.eq(source_ref.has_bounded_source_ref({
      kind = "external",
      ref = string.rep("x", 201),
    }, 200), false)
    t.eq(source_ref.has_bounded_source_ref({
      kind = "external",
    }, 200), false)
  end,
}
