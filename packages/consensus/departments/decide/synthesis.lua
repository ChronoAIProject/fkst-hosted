local M = {}
local provenance = require("departments.decide.provenance")
local strings = require("contract.strings")

local max_field_len = 1000
local max_findings_record_len = 1500
local max_findings_entry_len = 700
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

local function findings_line(value)
  local text = trim(value)
  if text == "" or #text > max_findings_entry_len then
    return nil
  end
  if text:find("%c") ~= nil then
    return nil
  end
  return text
end

local function parse_findings_value(value)
  local prefix, rest = trim(value):match("^([^:]+):%s*(.+)$")
  prefix = prefix and trim(prefix):lower() or nil
  rest = findings_line(rest)
  if rest == nil then
    return nil
  end
  if prefix == "settled" then
    local citation = trim(rest:match("%f[%a]by%s+refutation%s+of%f[%A]%s*(.+)$") or "")
    if citation == "" then
      return nil
    end
    return {
      kind = "settled",
      text = rest,
      citation = citation,
    }
  end
  if prefix == "settled-by-agreement (unverified)" then
    return {
      kind = "settled-by-agreement",
      text = rest,
    }
  end
  if prefix == "open" then
    return {
      kind = "open",
      text = rest,
    }
  end
  return nil
end

local function render_finding(entry, verified_citations)
  if type(entry) ~= "table" then
    return nil
  end
  if entry.kind == "settled" then
    if type(verified_citations) == "table" and verified_citations[entry.citation] then
      return "settled:\n" .. entry.text
    end
    return "settled-by-agreement (unverified):\n" .. entry.text
  end
  if entry.kind == "settled-by-agreement" then
    return "settled-by-agreement (unverified):\n" .. entry.text
  end
  if entry.kind == "open" then
    return "open:\n" .. entry.text
  end
  return nil
end

local function combine_findings_records(records, verified_citations)
  if #records == 0 then
    return nil
  end
  local rendered = {}
  for _, entry in ipairs(records) do
    local line = render_finding(entry, verified_citations)
    if line == nil then
      return nil
    end
    table.insert(rendered, line)
  end
  local text = table.concat(rendered, "\n")
  if #text > max_findings_record_len then
    return nil
  end
  return text
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

local function parse_premise_refuted(value, verdict_mode)
  if verdict_mode ~= "converge" then
    return nil
  end
  local framing = bounded_text(value, max_field_len)
  if framing == nil then
    return nil
  end
  return {
    kind = "reached",
    decision = "reject",
    decision_reason = "premise-refuted",
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

local function parse_essence_stall(value)
  local reason = bounded_text(value, max_field_len)
  if reason == nil then
    return nil
  end
  return M.essence_stall(reason)
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
  local text = trim(tostring(stdout or ""))
  if text:find("⟦FKST:PLAN⟧", 1, true) ~= nil then
    return nil
  end
  if text == "" then
    return nil
  end

  local parsed = nil
  local outcome_count = 0
  local verified_moves = {}
  local seen_verified_move = {}
  local findings = {}
  for line in (text .. "\n"):gmatch("(.-)\n") do
    local reached = line_with_prefix(line, "reached:")
    local premise_refuted = line_with_prefix(line, "premise-refuted:")
    local converge = line_with_prefix(line, "converge:")
    local essence_stall = line_with_prefix(line, "essence-stall:")
    local unresolvable = line_with_prefix(line, "unresolvable:")
    if reached ~= nil then
      outcome_count = outcome_count + 1
      parsed = parse_reached(reached, verdict_mode)
    elseif premise_refuted ~= nil then
      outcome_count = outcome_count + 1
      parsed = parse_premise_refuted(premise_refuted, verdict_mode)
    elseif converge ~= nil then
      outcome_count = outcome_count + 1
      parsed = parse_converge(converge)
    elseif essence_stall ~= nil then
      outcome_count = outcome_count + 1
      parsed = parse_essence_stall(essence_stall)
    elseif unresolvable ~= nil then
      outcome_count = outcome_count + 1
      parsed = parse_essence_stall(unresolvable)
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
      else
        local finding = parse_findings_value(line)
        if finding ~= nil then
          table.insert(findings, finding)
        elseif line:match("^%s*settled%s*:") ~= nil
          or line:match("^%s*settled%-by%-agreement%s*%([Uu][Nn][Vv][Ee][Rr][Ii][Ff][Ii][Ee][Dd]%)%s*:") ~= nil
          or line:match("^%s*open%s*:") ~= nil then
          return nil
        else
          return nil
        end
      end
    end
  end

  if outcome_count ~= 1 or parsed == nil then
    return nil
  end
  local findings_record = combine_findings_records(findings)
  if parsed.kind == "converge" and parsed.essence_stall ~= true and findings_record == nil then
    return nil
  end
  if findings_record ~= nil then
    parsed.findings_record = findings_record
  end
  parsed.findings_entries = findings
  parsed.verified_moves = #verified_moves
  parsed.verified_move_records = verified_moves
  return parsed
end

function M.essence_stall(disagreement)
  local text = findings_line(disagreement) or "synthesis produced no resolvable continuation"
  return {
    kind = "converge",
    disagreement = "essence-stall",
    resolving_evidence = "No concrete resolving evidence was named for: " .. text,
    narrowed_question = "essence-stall + No concrete resolving evidence was named for: " .. text,
    findings_record = "open:\n" .. text,
    essence_stall = true,
    verified_moves = 0,
  }
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

local function verified_move_citations(records, p1_results, p2_results)
  local count = 0
  local seen = {}
  local citations = {}
  for _, record in ipairs(records or {}) do
    local key = record.angle .. "\n" .. record.phase .. "\n" .. record.citation
    if not seen[key] then
      seen[key] = true
      if record.phase == "P1" and result_text_contains(p1_results, record.angle, record.citation) then
        count = count + 1
        citations[record.citation] = true
      elseif record.phase == "P2" and result_text_contains(p2_results, record.angle, record.citation) then
        count = count + 1
        citations[record.citation] = true
      end
    end
  end
  return citations, count
end

function M.count_verified_moves(records, p1_results, p2_results)
  local _, count = verified_move_citations(records, p1_results, p2_results)
  return count
end

local function stamp_verified_count(parsed, p1_results, p2_results)
  if parsed ~= nil then
    local citations, count = verified_move_citations(parsed.verified_move_records, p1_results, p2_results)
    parsed.verified_moves = count
    if parsed.findings_entries ~= nil then
      parsed.findings_record = combine_findings_records(parsed.findings_entries, citations)
    end
  end
  return parsed
end

local function fail_synthesis(first, repaired)
  local first_exit_code = type(first) == "table" and first.exit_code or nil
  local repair_exit_code = type(repaired) == "table" and repaired.exit_code or nil
  if repair_exit_code ~= 0 then
    local stderr = type(repaired) == "table" and trim(repaired.stderr) or ""
    local detail = stderr ~= "" and (" stderr=" .. stderr) or ""
    error(
      "consensus: codex-failed: phase=synthesis-repair"
        .. " first_exit_code=" .. tostring(first_exit_code)
        .. " repair_exit_code=" .. tostring(repair_exit_code)
        .. detail,
      0
    )
  end
  error(
    "consensus: synthesis-unparseable: phase=synthesis-repair"
      .. " first_exit_code=" .. tostring(first_exit_code)
      .. " repair_exit_code=" .. tostring(repair_exit_code),
    0
  )
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

  fail_synthesis(first, repaired)
end

function M.to_decision_result(proposal, p1_results, p2_results, parsed, caps)
  if parsed.kind == "reached" then
    caps.assert_all_angle_answers_valid(p1_results, "blind")
    caps.assert_all_angle_answers_valid(p2_results, "rebuttal")
    return {
      queue = "consensus_reached",
      payload = caps.build_reached_payload(proposal, {
        decision = parsed.decision,
        decision_reason = parsed.decision_reason,
      }, p2_results, parsed.framing, {
        verdict_path = "synthesis",
        p1_verdicts = provenance.verdict_vector(p1_results),
        p2_verdicts = provenance.verdict_vector(p2_results),
        verified_moves = parsed.verified_moves or 0,
      }),
    }
  end

  return {
    queue = "consensus_converge",
    angle_results = p2_results,
    narrowed_question = parsed.narrowed_question,
    findings_record = parsed.findings_record,
    essence_stall = parsed.essence_stall == true,
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
    table.insert(lines, "Verdict: " .. tostring(item and item.verdict))
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
