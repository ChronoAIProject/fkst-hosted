#!/usr/bin/env ruby
# frozen_string_literal: true

require "set"
require "yaml"

abort "usage: #{$PROGRAM_NAME} MONITORING_YAML BASE_YAML LOCAL_YAML" unless ARGV.length == 3

load_objects = lambda do |path|
  YAML.load_stream(File.read(path)).compact.map do |object|
    abort "#{path}: rendered document is not a mapping" unless object.is_a?(Hash)
    object
  end
end

monitoring = load_objects.call(ARGV.fetch(0))
base = load_objects.call(ARGV.fetch(1))
local = load_objects.call(ARGV.fetch(2))

identity = lambda do |object|
  [object.fetch("kind"), object.dig("metadata", "namespace").to_s,
   object.dig("metadata", "name").to_s]
end

find = lambda do |kind, name|
  object = monitoring.find { |item| identity.call(item) == [kind, "chronoai-fkst", name] }
  abort "monitoring render is missing #{kind}/#{name}" unless object
  object
end

expected = Set[
  ["Service", "chronoai-fkst", "fkst-control-plane-metrics"],
  ["ServiceMonitor", "chronoai-fkst", "fkst-control-plane"],
  ["PrometheusRule", "chronoai-fkst", "fkst-recovery"],
  ["Service", "chronoai-fkst", "fkst-audit-relay-metrics"],
  ["ServiceMonitor", "chronoai-fkst", "fkst-audit-relay"],
  ["PrometheusRule", "chronoai-fkst", "fkst-audit"]
]
actual = monitoring.map(&identity).to_set
abort "monitoring render must contain exactly the reviewed resources" unless actual == expected
abort "monitoring render must not contain Secret objects" if monitoring.any? { |object| object["kind"] == "Secret" }

base_namespace = base.find { |object| identity.call(object) == ["Namespace", "", "chronoai-fkst"] }
local_namespace = local.find { |object| identity.call(object) == ["Namespace", "", "chronoai-fkst"] }
abort "base render is missing chronoai-fkst" unless base_namespace
abort "local render is missing chronoai-fkst" unless local_namespace
disposable_label = "fkst.chronoai.io/disposable"
abort "base namespace must remain production-neutral" if base_namespace.dig("metadata", "labels", disposable_label)
unless local_namespace.dig("metadata", "labels", disposable_label) == "true"
  abort "local namespace must be explicitly disposable"
end
other_disposable = local.select do |object|
  object["kind"] == "Namespace" && object.dig("metadata", "labels", disposable_label)
end.reject { |object| object.dig("metadata", "name") == "chronoai-fkst" }
abort "only chronoai-fkst may be disposable in the local overlay" unless other_disposable.empty?

service = find.call("Service", "fkst-control-plane-metrics")
unless service.dig("spec", "selector") == { "app.kubernetes.io/name" => "fkst-control-plane" }
  abort "metrics Service must scrape both control-plane contenders"
end
relay_service = find.call("Service", "fkst-audit-relay-metrics")
unless relay_service.dig("spec", "selector") == { "app.kubernetes.io/name" => "fkst-audit-relay" }
  abort "audit-relay metrics Service selection drifted"
end

# One reviewed endpoint contract per monitor: a fixed namespace, a fixed target
# label, and a bounded interval. `honorLabels: false` keeps a scraped body from
# overwriting the namespace/service labels every alert expression pins.
check_monitor = lambda do |monitor, target_label|
  name = monitor.dig("metadata", "name")
  unless monitor.dig("spec", "namespaceSelector", "matchNames") == ["chronoai-fkst"]
    abort "#{name} ServiceMonitor namespace selection drifted"
  end
  unless monitor.dig("spec", "selector", "matchLabels") == { "app.kubernetes.io/name" => target_label }
    abort "#{name} ServiceMonitor target selection drifted"
  end
  endpoints = monitor.dig("spec", "endpoints")
  abort "#{name} ServiceMonitor must expose one bounded endpoint" unless endpoints.is_a?(Array) && endpoints.length == 1
  endpoint = endpoints.first
  unless endpoint.slice("port", "path", "interval", "scrapeTimeout", "honorLabels") == {
    "port" => "http", "path" => "/metrics", "interval" => "30s",
    "scrapeTimeout" => "10s", "honorLabels" => false
  }
    abort "#{name} ServiceMonitor endpoint contract drifted"
  end
  endpoint
end

monitor = find.call("ServiceMonitor", "fkst-control-plane")
endpoint = check_monitor.call(monitor, "fkst-control-plane-metrics")
expected_drop = [{
  "sourceLabels" => ["__name__"],
  "regex" => "fkst_leader_identity_info|fkst_leader_observed_holder_info",
  "action" => "drop"
}]
abort "identity-bearing metrics must be dropped before ingestion" unless endpoint["metricRelabelings"] == expected_drop

relay_monitor = find.call("ServiceMonitor", "fkst-audit-relay")
relay_endpoint = check_monitor.call(relay_monitor, "fkst-audit-relay-metrics")
# The relay publishes no info metric carrying an identity, so it needs no drop
# list — and must not silently acquire one that nobody reviewed.
abort "audit-relay scrape must not carry an unreviewed relabel rule" if relay_endpoint.key?("metricRelabelings")

CONTROL_PLANE_SERVICE = "fkst-control-plane-metrics"
RELAY_SERVICE = "fkst-audit-relay-metrics"

# Substrings that would mean a series, label, or notification carries an
# identity. Alert text is delivered to humans through systems this repository
# does not control, so the check is on the literal rendered strings (epic
# `OPS-04`).
FORBIDDEN_IDENTITY_TOKENS = %w[
  actor login repository session_id runtime_id request_id event_id
  cursor viewer installation issue_number
].freeze

# One group per PrometheusRule, validated the same way: a fixed identity, a
# closed alert set, an allowlisted metric vocabulary, the exact scrape scope for
# each metric used, a bounded hold duration, and static notifications.
validate_group = lambda do |rules_object, group_name, expected_alerts, metric_services, components, rule_count|
  groups = rules_object.dig("spec", "groups")
  abort "#{group_name} PrometheusRule must contain one fixed group" unless groups.is_a?(Array) && groups.length == 1
  group = groups.first
  unless group.slice("name", "interval") == { "name" => group_name, "interval" => "30s" }
    abort "#{group_name} PrometheusRule group identity drifted"
  end
  alert_rules = group.fetch("rules")
  unless alert_rules.length == rule_count
    abort "#{group_name} must contain exactly #{rule_count} reviewed rules"
  end
  unless alert_rules.map { |rule| rule["alert"] }.to_set == expected_alerts
    abort "#{group_name} alert set drifted"
  end
  alert_rules.each do |rule|
    name = rule.fetch("alert")
    expression = rule.fetch("expr").to_s
    metrics = expression.scan(/\bfkst_[a-z0-9_]+\b/).to_set
    unknown = metrics - metric_services.keys.to_set
    abort "#{name} uses an unreviewed metric" unless unknown.empty?
    abort "#{name} lacks the fixed namespace scope" unless expression.include?('namespace="chronoai-fkst"')
    # Every metric must be read from the target that actually publishes it. The
    # control plane and the relay share the `fkst_audit_relay_` prefix (client
    # side vs storage side), so an expression missing this pin would aggregate
    # two unrelated families.
    required_services = metrics.map { |metric| metric_services.fetch(metric) }.to_set
    used_services = expression.scan(/service="([a-z0-9-]+)"/).flatten.to_set
    unless used_services == required_services
      abort "#{name} does not pin exactly the scrape targets its metrics come from"
    end
    abort "#{name} must have a bounded hold duration" if rule["for"].to_s.empty?
    labels = rule.fetch("labels")
    unless labels.keys.to_set == Set["component", "severity"]
      abort "#{name} labels are not the fixed reviewed set"
    end
    abort "#{name} carries an unreviewed component" unless components.include?(labels["component"])
    unless %w[warning critical].include?(labels["severity"])
      abort "#{name} severity must be warning or critical"
    end
    annotations = rule.fetch("annotations")
    unless annotations.keys.to_set == Set["summary", "description", "runbook_url"]
      abort "#{name} annotations are not the fixed reviewed set"
    end
    if (labels.values + annotations.values).any? { |value| value.to_s.include?("{{") }
      abort "#{name} must not project dynamic labels into notifications"
    end
    unless annotations["runbook_url"].to_s.start_with?("https://")
      abort "#{name} must link a runbook"
    end
    notification = (annotations.values + labels.values).join(" ").downcase
    FORBIDDEN_IDENTITY_TOKENS.each do |token|
      abort "#{name} notification text mentions #{token}" if notification.include?(token)
    end
  end
  # Two rules may share an alert name only to express a warning/critical pair.
  alert_rules.group_by { |rule| rule["alert"] }.each do |name, rules|
    next if rules.length == 1
    severities = rules.map { |rule| rule.dig("labels", "severity") }
    unless severities.uniq.length == severities.length
      abort "#{name} repeats an alert name without distinct severities"
    end
  end
end

recovery_metrics = {
  "fkst_up" => CONTROL_PLANE_SERVICE,
  "fkst_startup_resync_complete" => CONTROL_PLANE_SERVICE,
  "fkst_startup_resync_last_success_timestamp_seconds" => CONTROL_PLANE_SERVICE,
  "fkst_leader_state" => CONTROL_PLANE_SERVICE,
  "fkst_leader_election_enabled" => CONTROL_PLANE_SERVICE,
  "fkst_leader_ready" => CONTROL_PLANE_SERVICE,
  "fkst_leader_routing_ready" => CONTROL_PLANE_SERVICE,
  "fkst_leader_lease_failures_total" => CONTROL_PLANE_SERVICE,
  "fkst_leader_routing_failures_total" => CONTROL_PLANE_SERVICE,
  "fkst_leader_transitions_total" => CONTROL_PLANE_SERVICE
}
recovery_alerts = Set[
  "FKSTControlPlaneScrapeMissing",
  "FKSTStartupResyncIncomplete",
  "FKSTRecoveryDegraded",
  "FKSTRecoveryStale",
  "FKSTNoReadyLeader",
  "FKSTLeaderRoutingUnavailable",
  "FKSTLeaderLeaseFailures",
  "FKSTLeaderRoutingFailures",
  "FKSTExcessiveLeaderChurn"
]
validate_group.call(find.call("PrometheusRule", "fkst-recovery"), "fkst.recovery",
                    recovery_alerts, recovery_metrics, Set["control-plane"], 9)

audit_metrics = {
  "fkst_audit_relay_max_records" => RELAY_SERVICE,
  "fkst_audit_required_rejections_total" => CONTROL_PLANE_SERVICE,
  "fkst_operations_activity_queries_total" => CONTROL_PLANE_SERVICE,
  "fkst_operations_activity_source_partial_total" => CONTROL_PLANE_SERVICE,
  "fkst_operations_sandbox_inventory_requests_total" => CONTROL_PLANE_SERVICE,
  "fkst_session_access_registry_generation_state" => CONTROL_PLANE_SERVICE,
  "fkst_audit_relay_ingress_ready" => RELAY_SERVICE,
  "fkst_audit_relay_records" => RELAY_SERVICE,
  "fkst_audit_relay_oldest_record_age_seconds" => RELAY_SERVICE,
  "fkst_audit_relay_dead_letters_total" => RELAY_SERVICE,
  "fkst_audit_relay_incomplete_total" => RELAY_SERVICE,
  "fkst_audit_relay_db_bytes" => RELAY_SERVICE
}
audit_alerts = Set[
  "FKSTAuditIngressUnavailable",
  "FKSTAuditRelayNotReady",
  "FKSTAuditBacklogGrowing",
  "FKSTAuditPostHogUnverified",
  "FKSTAuditDeadLetters",
  "FKSTAuditIncompleteRequests",
  "FKSTAuditRelayCapacityPressure",
  "FKSTAuditRelayDiskPressure",
  "FKSTOperationsActivityQueryFailures",
  "FKSTSandboxInventoryFailures",
  "FKSTSessionVisibilityNotReady"
]
validate_group.call(find.call("PrometheusRule", "fkst-audit"), "fkst.audit",
                    audit_alerts, audit_metrics, Set["control-plane", "audit-relay"], 13)

# ------------------------------------------------------------ runbook anchors
#
# An alert whose runbook_url points at a heading that does not exist sends the
# on-call operator to the top of a 500-line document at exactly the moment they
# needed one procedure. Slugs are derived by GitHub from the heading text, so
# renaming a heading silently breaks every link to it; nothing else in the repo
# would notice. The same check covers the two documents' links to each other.
DOC_DIR = __dir__
RUNBOOKS = %w[AUDIT-RUNBOOK.md RECOVERY-RUNBOOK.md AUDIT-TRACE.md].freeze

# GitHub's rule: downcase, drop everything that is not a word character, a
# space, or a hyphen, then turn spaces into hyphens.
slugify = lambda do |heading|
  heading.downcase.gsub(/[^\w\s-]/, "").strip.gsub(/\s+/, "-")
end

anchors = RUNBOOKS.to_h do |name|
  path = File.join(DOC_DIR, name)
  abort "runbook #{name} is missing" unless File.file?(path)
  headings = File.readlines(path).grep(/\A#{'#'}{1,6} /).map { |line| slugify.call(line.sub(/\A#+ /, "")) }
  [name, headings.to_set]
end

check_anchor = lambda do |source, document, anchor|
  return if anchor.nil? || anchor.empty?
  known = anchors[document]
  abort "#{source} links an unknown document #{document}" unless known
  abort "#{source} links #{document}##{anchor}, which is not a heading" unless known.include?(anchor)
end

monitoring.each do |object|
  next unless object["kind"] == "PrometheusRule"
  (object.dig("spec", "groups") || []).each do |group|
    group.fetch("rules").each do |rule|
      url = rule.dig("annotations", "runbook_url").to_s
      document, anchor = url.split("#", 2)
      check_anchor.call("alert #{rule['alert']}", File.basename(document), anchor)
    end
  end
end

RUNBOOKS.each do |name|
  File.read(File.join(DOC_DIR, name)).scan(/\]\(([A-Z][A-Za-z-]+\.md)?#([a-z0-9-]+)\)/) do |document, anchor|
    check_anchor.call("#{name} link", document || name, anchor)
  end
end

puts "validated optional recovery/audit monitoring, runbook anchors, and the local-only disposable boundary"
