local S = {}

local classes = { "expedite", "standard", "background" }
local class_set = {
  expedite = true,
  standard = true,
  background = true,
}

function S.install(M)
function M.normalize_intake_service_class(value)
  local text = tostring(value or ""):lower()
  if class_set[text] then
    return text
  end
  return "standard"
end

function M.is_intake_service_class(value)
  return class_set[tostring(value or "")] == true
end

function M.intake_service_class_label(value)
  return "fkst-class:" .. M.normalize_intake_service_class(value)
end

function M.intake_service_class_labels()
  local labels = {}
  for _, class in ipairs(classes) do
    table.insert(labels, M.intake_service_class_label(class))
  end
  return labels
end

function M.intake_service_class_label_changes(value)
  local class = M.normalize_intake_service_class(value)
  local add = { M.intake_service_class_label(class) }
  local remove = {}
  for _, candidate in ipairs(classes) do
    if candidate ~= class then
      table.insert(remove, M.intake_service_class_label(candidate))
    end
  end
  return add, remove
end

end

return S
