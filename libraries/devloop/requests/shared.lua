local S = {}
local comment_strings = require("devloop.strings")

function S.new(M)
local shared = {}
local strings = require("contract.strings")
local ai_sentinel = "⟦AI:FKST⟧"
local display_separator = " — "
local max_display_question_len = 2000
local max_display_digest_len = 600
local max_display_attr_len = 120
local max_display_block_len = 5000
local max_verdict_summary_items = 8
local max_verdict_summary_len = 600
local function bounded_neutralized_text(value, limit)
  local text = tostring(value or "")
  local cap = limit or max_display_digest_len
  if #text > cap then
    text = M.truncate_utf8(text, cap)
  end
  text = M.neutralize_untrusted_comment_text(text)
  if #text > cap then
    text = M.truncate_utf8(text, cap)
  end
  return text
end

local function angle_display_text(item)
  if type(item) ~= "table" then
    return nil
  end
  local angle = bounded_neutralized_text(item.angle or "unknown", max_display_attr_len)
  local verdict = bounded_neutralized_text(item.verdict or "invalid", max_display_attr_len)
  local digest = item.digest
  if digest == nil or tostring(digest) == "" then
    digest = item.reply
  end
  digest = bounded_neutralized_text(digest or "", max_display_digest_len)
  if digest == "" then
    return "- " .. angle .. ": " .. verdict
  end
  return "- " .. angle .. ": " .. verdict .. display_separator .. digest
end

local function build_convergence_display(header, unresolved, round)
  local lines = {
    header .. tostring(round) .. comment_strings.comment_string(M, "convergence_suffix"),
  }
  local question = bounded_neutralized_text(unresolved and unresolved.narrowed_question or "", max_display_question_len)
  if question ~= "" then
    table.insert(lines, "")
    table.insert(lines, comment_strings.comment_string(M, "narrowed_question_label") .. question)
  end
  local angle_lines = {}
  if type(unresolved) == "table" and type(unresolved.angle_digests) == "table" then
    for _, item in ipairs(unresolved.angle_digests) do
      local line = angle_display_text(item)
      if line ~= nil then
        table.insert(angle_lines, line)
      end
    end
  end
  if #angle_lines > 0 then
    table.insert(lines, "")
    table.insert(lines, comment_strings.comment_string(M, "angle_stances_label"))
    for _, line in ipairs(angle_lines) do
      table.insert(lines, line)
    end
  end
  local body = table.concat(lines, "\n")
  if #body > max_display_block_len then
    body = M.truncate_utf8(body, max_display_block_len)
  end
  return body
end

local function build_verdict_summary(angle_results)
  if type(angle_results) ~= "table" then
    return nil
  end
  local parts = {}
  for _, item in ipairs(angle_results) do
    if #parts >= max_verdict_summary_items then
      break
    end
    if type(item) == "table" then
      local angle = bounded_neutralized_text(item.angle or "unknown", max_display_attr_len)
      local verdict = bounded_neutralized_text(item.verdict or "invalid", max_display_attr_len)
      table.insert(parts, angle .. "=" .. verdict)
    end
  end
  if #parts == 0 then
    return nil
  end
  local summary = comment_strings.comment_string(M, "verdict_summary_label") .. table.concat(parts, " ")
  if #summary > max_verdict_summary_len then
    summary = M.truncate_utf8(summary, max_verdict_summary_len)
  end
  return summary
end

local function bounded_blocking_gap(M, reached)
  local gap = reached and reached.blocking_gap
  if gap == nil and type(reached and reached.blocking_gaps) == "table" then
    gap = reached.blocking_gaps[1]
  end
  local text = tostring(gap or ""):gsub("%c", " "):gsub("%s+", " ")
  text = text:gsub("^%s+", ""):gsub("%s+$", "")
  if text == "" then
    return nil
  end
  if #text > M._max_blocking_gap_len then
    text = M.truncate_utf8(text, M._max_blocking_gap_len)
  end
  return text
end

shared.strings = strings
shared.ai_sentinel = ai_sentinel
shared.display_separator = display_separator
shared.max_display_question_len = max_display_question_len
shared.max_display_digest_len = max_display_digest_len
shared.max_display_attr_len = max_display_attr_len
shared.max_display_block_len = max_display_block_len
shared.max_verdict_summary_items = max_verdict_summary_items
shared.max_verdict_summary_len = max_verdict_summary_len
shared.bounded_neutralized_text = bounded_neutralized_text
shared.build_convergence_display = build_convergence_display
shared.build_verdict_summary = build_verdict_summary
shared.bounded_blocking_gap = bounded_blocking_gap
return shared
end

return S
