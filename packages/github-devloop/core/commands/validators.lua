local S = {}

function S.bounded_limit(value, fallback, minimum, maximum, message)
  local n = tonumber(value or fallback)
  if n == nil or n < minimum or n > maximum then
    error(message)
  end
  return math.floor(n)
end

function S.validate_fields(fields, message)
  local selected_fields = tostring(fields or "")
  if selected_fields == "" or selected_fields:match("[^%w_,]") or selected_fields:match("^,") or selected_fields:match(",$") or selected_fields:match(",,") then
    error(message)
  end
  return selected_fields
end

function S.require_safe_branch(M, name, value)
  if not M._is_git_ref_safe(value) then
    error("github-devloop: invalid " .. tostring(name))
  end
  return tostring(value)
end

function S.require_safe_remote(M, remote)
  local value = tostring(remote or "")
  if value == "" or value:find("[\r\n]") ~= nil then
    error("github-devloop: invalid git remote")
  end
  if not M._is_git_ref_safe(value) then
    error("github-devloop: invalid git remote")
  end
  return value
end

function S.require_safe_sha(M, name, value)
  if not M._is_git_sha(value) then
    error("github-devloop: invalid " .. tostring(name))
  end
  return tostring(value)
end

function S.require_positive_pr_number(M, value)
  if not M._is_positive_pr_number(value) then
    error("github-devloop: invalid pull request number")
  end
  return tostring(value)
end

function S.require_label_name(name)
  local value = tostring(name or "")
  if value == "" then
    error("github-devloop: label name is required")
  end
  return value
end

function S.require_label_color(color)
  local value = tostring(color or "")
  if value:find("^%x%x%x%x%x%x$") == nil then
    error("github-devloop: label color is invalid")
  end
  return value
end

function S.require_dashboard_label(label)
  local value = tostring(label or "")
  if value == "" then
    error("github-devloop: dashboard issue label is required")
  end
  return value
end

function S.install()
end

return S
