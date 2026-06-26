local S = {}

local classes = { "expedite", "standard", "background" }

function S.install(M)
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

function M.build_intake_service_class_label_request(repo, issue_number, candidate)
  local add_labels, remove_labels = M.intake_service_class_label_changes(candidate and candidate.service_class)
  return M.build_label_request(
    repo,
    issue_number,
    add_labels,
    remove_labels,
    M._dedup_key({
      "intake",
      "class-label",
      tostring(candidate and candidate.proposal_id or ""),
      tostring(candidate and candidate.dedup_key or ""),
    }),
    candidate and candidate.source_ref
  )
end

end

return S
