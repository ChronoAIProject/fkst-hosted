local h = require("tests.devloop_helpers")
local t = h.t

local source_path = "packages/github-devloop-pr/departments/comment_handoff/main.lua"

local function source()
  return file.read(source_path)
end

local function strategy_entry(body, kind)
  local start_pos = body:find('["' .. kind .. '"] = {', 1, true)
  if start_pos == nil then
    return nil
  end
  local next_pos = body:find('\n  %["github%-devloop%.', start_pos + 1)
  local table_end = body:find("\n}", start_pos + 1, true)
  local end_pos = next_pos or table_end or #body
  return body:sub(start_pos, end_pos)
end

return {
  test_comment_handoff_variants_are_owned_by_one_strategy_table = function()
    local body = source()
    t.is_true(body:find("local handoff_strategies = {", 1, true) ~= nil)

    for _, kind in ipairs({
      "github-devloop.pr_open",
      "github-devloop.reviewing",
      "github-devloop.blocked",
      "github-devloop.closed_unmerged",
      "github-devloop.merge_ready",
      "github-devloop.fixing",
    }) do
      local entry = strategy_entry(body, kind)
      t.is_true(entry ~= nil)
      t.is_true(entry:find("validate =", 1, true) ~= nil)
      t.is_true(entry:find("state =", 1, true) ~= nil)
      t.is_true(entry:find("emit =", 1, true) ~= nil)
    end
  end,
}
