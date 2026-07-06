local M = {}
local provenance = require("departments.decide.provenance")
local strings = require("contract.strings")

local max_field_len = 1000
local max_narrowed_question_len = 2000
local max_verified_moves = 64
local max_mover_len = 240

local trim = strings.trim

local function bounded_text(value, limit)
  local text = trim(value)
  if text == "" or #text > limit then
    return nil
  end
  return text
end

local function plain_line_count(text)
  local count = 0
  for _ in (tostring(text or "") .. "\n"):gmatch("(.-)\n") do
    count = count + 1
  end
  return count
end

local function count_literal(text, needle)
  local total = 0
  local start = 1
  while true do
    local found = tostring(text or ""):find(needle, start, true)
    if found == nil then
      return total
    end
    total = total + 1
    start = found + #needle
  end
end

local function line_with_prefix(line, prefix)
  local text = tostring(line or ""):gsub("^%s+", "")
  if text:sub(1, #prefix):lower() == prefix then
    return trim(text:sub(#prefix + 1))
  end
  return nil
end

local function parse_reached(value, verdict_mode)
  local first, framing = trim(value):match("^(%S+)%s+(.+)$")
  local decision = first and first:lower() or nil
  if decision ~= "approve" and not (verdict_mode == "gate" and decision == "reject") then
    return nil
  end
  framing = bounded_text(framing, max_field_len)
  if framing == nil then
    return nil
  end
  return {
    kind = "reached",
    decision = decision,
    framing = framing,
  }
end

local function parse_converge(value)
  local disagreement, evidence = trim(value):match("^([^+]+)%s+%+%s+(.+)$")
  disagreement = bounded_text(disagreement, max_field_len)
  evidence = bounded_text(evidence, max_field_len)
  if disagreement == nil or evidence == nil then
    return nil
  end
  return {
    kind = "converge",
    disagreement = disagreement,
    resolving_evidence = evidence,
    narrowed_question = disagreement .. " + " .. evidence,
  }
end

local function parse_verified_move(line)
  local value = line:match("^%s*verified%-move:%s*(.+)%s*$")
  if value == nil then
    return nil
  end
  value = bounded_text(value, max_mover_len)
  if value == nil then
    return nil
  end
  local angle, phase, citation = value:match("^angle=([%w._-]+)%s+phase=(P[12])%s+citation=(.+)$")
  citation = bounded_text(citation, max_mover_len)
  if angle == nil or phase == nil or citation == nil then
    return nil
  end
  return {
    angle = angle,
    phase = phase,
    citation = citation,
  }
end

function M.parse_output(stdout, verdict_mode)
  local text = tostring(stdout or "")
  if text:find("⟦FKST:PLAN⟧", 1, true) ~= nil then
    return nil
  end

  local parsed = nil
  local outcome_count = 0
  local verified_moves = {}
  local seen_verified_move = {}
  for line in (text .. "\n"):gmatch("(.-)\n") do
    local reached = line_with_prefix(line, "reached:")
    local converge = line_with_prefix(line, "converge:")
    if reached ~= nil then
      outcome_count = outcome_count + 1
      parsed = parse_reached(reached, verdict_mode)
    elseif converge ~= nil then
      outcome_count = outcome_count + 1
      parsed = parse_converge(converge)
    else
      local move = parse_verified_move(line)
      if move ~= nil then
        local move_key = move.angle .. "\n" .. move.phase .. "\n" .. move.citation
        if seen_verified_move[move_key] then
          return nil
        end
        seen_verified_move[move_key] = true
        table.insert(verified_moves, move)
        if #verified_moves > max_verified_moves then
          return nil
        end
      elseif line:match("^%s*verified%-move:") ~= nil then
        return nil
      end
    end
  end

  if outcome_count ~= 1 or parsed == nil then
    return nil
  end
  parsed.verified_moves = #verified_moves
  parsed.verified_move_records = verified_moves
  return parsed
end

local function result_text_contains(results, angle, citation)
  for _, item in ipairs(results or {}) do
    if tostring(item.angle or "") == angle then
      if tostring(item.stdout or ""):find(citation, 1, true) ~= nil then
        return true
      end
      if tostring(item.peer_claim or "") == citation then
        return true
      end
    end
  end
  return false
end

function M.count_verified_moves(records, p1_results, p2_results)
  local count = 0
  local seen = {}
  for _, record in ipairs(records or {}) do
    local key = record.angle .. "\n" .. record.phase .. "\n" .. record.citation
    if not seen[key] then
      seen[key] = true
      if record.phase == "P1" and result_text_contains(p1_results, record.angle, record.citation) then
        count = count + 1
      elseif record.phase == "P2" and result_text_contains(p2_results, record.angle, record.citation) then
        count = count + 1
      end
    end
  end
  return count
end

local function stamp_verified_count(parsed, p1_results, p2_results)
  if parsed ~= nil then
    parsed.verified_moves = M.count_verified_moves(parsed.verified_move_records, p1_results, p2_results)
  end
  return parsed
end

function M.parse_or_retry(ctx)
  local first = ctx.spawn_sync("synthesis", ctx.build_prompt(false))
  local parsed = nil
  if type(first) == "table" and first.exit_code == 0 then
    parsed = stamp_verified_count(M.parse_output(first.stdout, ctx.verdict_mode), ctx.p1_results, ctx.p2_results)
  end
  if parsed ~= nil then
    return parsed
  end

  local repaired = ctx.spawn_sync("synthesis-repair", ctx.build_prompt(true, first))
  if type(repaired) == "table" and repaired.exit_code == 0 then
    parsed = stamp_verified_count(M.parse_output(repaired.stdout, ctx.verdict_mode), ctx.p1_results, ctx.p2_results)
  end
  if parsed ~= nil then
    return parsed
  end

  return {
    kind = "converge",
    disagreement = "synthesis-parse-failed",
    resolving_evidence = "Provide one valid synthesis outcome using reached:<decision> <framing> or converge:<named disagreement> + <concrete resolving evidence>.",
    narrowed_question = "synthesis-parse-failed + Provide one valid synthesis outcome using reached:<decision> <framing> or converge:<named disagreement> + <concrete resolving evidence>.",
    verified_moves = 0,
  }
end

function M.to_decision_result(proposal, p1_results, p2_results, parsed, caps)
  if parsed.kind == "reached" then
    if not caps.all_angles_succeeded(p1_results) or not caps.all_angles_succeeded(p2_results) then
      return {
        queue = "consensus_converge",
        angle_results = p2_results,
        narrowed_question = "synthesis-parse-failed + Re-run synthesis after every Phase R result succeeds.",
      }
    end
    return {
      queue = "consensus_reached",
      payload = caps.build_reached_payload(proposal, parsed.decision, p2_results, parsed.framing, {
        verdict_path = "synthesis",
        p1_verdicts = provenance.verdict_vector(p1_results),
        p2_verdicts = provenance.verdict_vector(p2_results),
        verified_moves = parsed.verified_moves or 0,
      }),
      cache = true,
    }
  end

  return {
    queue = "consensus_converge",
    angle_results = p2_results,
    narrowed_question = parsed.narrowed_question,
  }
end

function M.build_prompt(ctx, repair, prior_result)
  local prompt = require("prompts.synthesis")
  local vars = ctx.vars(repair, prior_result)
  return ctx.render_prompt_template(prompt.template, vars, ctx.proposal)
end

function M.full_transcript_lines(neutralize, label, results)
  local lines = { label }
  for _, item in ipairs(results or {}) do
    table.insert(lines, "Angle: " .. neutralize(item and item.angle))
    table.insert(lines, "Verdict: " .. tostring(item and item.verdict or "invalid"))
    table.insert(lines, "Exit code: " .. tostring(item and item.exit_code or "nil"))
    table.insert(lines, "Full output (" .. tostring(plain_line_count(item and item.stdout or "")) .. " lines):")
    table.insert(lines, neutralize(item and item.stdout or ""))
    table.insert(lines, "")
  end
  if #lines > 1 then
    table.remove(lines)
  end
  return table.concat(lines, "\n")
end

function M.verified_move_candidates(p2_results)
  local candidates = {}
  for _, item in ipairs(p2_results or {}) do
    if item.stance == "update" and type(item.peer_claim) == "string" and item.peer_claim ~= "" then
      table.insert(candidates, "angle=" .. tostring(item.angle) .. " phase=P2 citation=" .. item.peer_claim)
    end
  end
  if #candidates == 0 then
    return "No Phase R update movers were parsed. Do not emit verified-move lines unless your own graft rests on a citation you can verify inside the supplied transcripts and manifest."
  end
  return table.concat(candidates, "\n")
end

M.count_literal = count_literal

return M
