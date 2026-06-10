local S = {}

function S.install(M)
local function normalized_text(value)
  return tostring(value or ""):lower()
end

local function has_any_label(labels, names)
  for _, label in ipairs(labels or {}) do
    if names[normalized_text(label)] then
      return true
    end
  end
  return false
end

local function has_word(text, word)
  return text:find("%f[%w]" .. word .. "%f[%W]") ~= nil
end

local function has_tracker_word(text)
  return has_word(text, "umbrella")
    or has_word(text, "epic")
    or has_word(text, "tracker")
    or has_word(text, "tracking")
end

local function has_wave_index(text)
  local count = 0
  for _ in text:gmatch("%f[%w]wave[%s%-_]*%d+%f[%W]") do
    count = count + 1
    if count >= 2 then
      return true
    end
  end
  return false
end

local function asks_to_split(text)
  return text:find("split into", 1, true) ~= nil
    or text:find("decomposed", 1, true) ~= nil
    or text:find("independent wave", 1, true) ~= nil
end

local function explicitly_non_implementable(text)
  return text:find("do not auto%-implement") ~= nil
    or text:find("not auto%-implement") ~= nil
    or text:find("non%-implementable") ~= nil
    or text:find("intake:%s*decline") ~= nil
end

function M.static_intake_decision(current)
  local title = normalized_text(current and current.title)
  local body = normalized_text(current and current.body)
  local joined = title .. "\n" .. body
  local tracker_labels = {
    umbrella = true,
    epic = true,
  }

  if title:find("^%s*%[umbrella%]") ~= nil
    or has_any_label(current and current.labels, tracker_labels)
    or (has_tracker_word(joined) and (has_wave_index(joined) or asks_to_split(joined) or explicitly_non_implementable(joined))) then
    return {
      action = "decline",
      reason = "Umbrella or epic tracker issues are not directly implementable; split them into independent wave proposals.",
    }
  end

  return nil
end
end

return S
