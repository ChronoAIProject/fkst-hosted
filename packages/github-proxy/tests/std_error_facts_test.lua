-- std module behavior tests are hosted in github-proxy (a flat package, the
-- strictest single-root conformance gate) because the engine test runner only
-- scans <root>/tests and <root>/departments/* (no recursion into std/tests).
local facts = require("std.error_facts")
local t = fkst.test

return {
  test_normalized_message_removes_timestamp_sha_and_tmp_path_noise = function()
    local first = facts.normalized_message("FAIL at 2026-06-11T20:57:25Z in /tmp/fkst-a/run sha 81bb199f4a3eda6d736d11100856a12230030b0e")
    local second = facts.normalized_error_message("fail at 2026-06-12T01:02:03Z in /tmp/fkst-b/run sha 7d9c0a1b2c3d4e5f678901234567890abcdef123")

    t.eq(first, "fail at <time>z in <path> sha <sha>")
    t.eq(second, first)
  end,

  test_stable_hash_uses_existing_fp_prefix_and_hash_algorithm = function()
    t.eq(facts.stable_hash("caught-failure|queue|dept|message"), "fp-1571597685")
    t.eq(facts.stable_hash("caught-failure|queue|dept|message"), facts.stable_hash("caught-failure|queue|dept|message"))
  end,

  test_source_ref_field_compacts_tables_and_strings = function()
    t.eq(facts.source_ref_field({ kind = "external", ref = "owner/repo#issue/42" }), "external:owner/repo#issue/42")
    t.eq(facts.source_ref_field("raw\nref"), "raw ref")
    t.is_nil(facts.source_ref_field(nil))
  end,
}
