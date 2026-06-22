local S = {}

function S.install(M)
local strings = require("contract.strings")
local github_view = require("forge.github_view")
local label_names = github_view.label_names

local function each_paginated_item(decoded, callback)
  if type(decoded) ~= "table" then
    return
  end
  for _, value in ipairs(decoded) do
    if type(value) == "table" then
      if value[1] ~= nil then
        for _, item in ipairs(value) do
          callback(item)
        end
      elseif next(value) ~= nil then
        callback(value)
      end
    end
  end
end

local function parse_numbered_list(stdout)
  local decoded = json.decode(stdout or "[]")
  local items = {}
  each_paginated_item(decoded, function(item)
    if type(item) == "table" and tonumber(item.number) ~= nil then
      table.insert(items, {
        number = tonumber(item.number),
        state = item.state,
        updated_at = item.updated_at or item.updatedAt,
      })
    end
  end)
  return items
end

return {
  strings = strings,
  github_view = github_view,
  label_names = label_names,
  each_paginated_item = each_paginated_item,
  parse_numbered_list = parse_numbered_list,
}
end

return S
