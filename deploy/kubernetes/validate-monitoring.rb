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

expected = Set[
  ["Service", "chronoai-fkst", "fkst-control-plane-metrics"],
  ["ServiceMonitor", "chronoai-fkst", "fkst-control-plane"],
  ["PrometheusRule", "chronoai-fkst", "fkst-recovery"]
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

service = monitoring.find { |object| object["kind"] == "Service" }
unless service.dig("spec", "selector") == { "app.kubernetes.io/name" => "fkst-control-plane" }
  abort "metrics Service must scrape both control-plane contenders"
end

monitor = monitoring.find { |object| object["kind"] == "ServiceMonitor" }
unless monitor.dig("spec", "namespaceSelector", "matchNames") == ["chronoai-fkst"]
  abort "ServiceMonitor namespace selection drifted"
end
unless monitor.dig("spec", "selector", "matchLabels") == {
  "app.kubernetes.io/name" => "fkst-control-plane-metrics"
}
  abort "ServiceMonitor target selection drifted"
end
endpoints = monitor.dig("spec", "endpoints")
abort "ServiceMonitor must expose one bounded endpoint" unless endpoints.is_a?(Array) && endpoints.length == 1
endpoint = endpoints.first
unless endpoint.slice("port", "path", "interval", "scrapeTimeout", "honorLabels") == {
  "port" => "http", "path" => "/metrics", "interval" => "30s",
  "scrapeTimeout" => "10s", "honorLabels" => false
}
  abort "ServiceMonitor endpoint contract drifted"
end
expected_drop = [{
  "sourceLabels" => ["__name__"],
  "regex" => "fkst_leader_identity_info|fkst_leader_observed_holder_info",
  "action" => "drop"
}]
abort "identity-bearing metrics must be dropped before ingestion" unless endpoint["metricRelabelings"] == expected_drop

rules = monitoring.find { |object| object["kind"] == "PrometheusRule" }
groups = rules.dig("spec", "groups")
abort "PrometheusRule must contain one fixed group" unless groups.is_a?(Array) && groups.length == 1
group = groups.first
abort "PrometheusRule group identity drifted" unless group.slice("name", "interval") == {
  "name" => "fkst.recovery", "interval" => "30s"
}

expected_alerts = Set[
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
alert_rules = group.fetch("rules")
abort "recovery alert set drifted" unless alert_rules.map { |rule| rule["alert"] }.to_set == expected_alerts

allowed_metrics = Set[
  "fkst_up",
  "fkst_startup_resync_complete",
  "fkst_startup_resync_last_success_timestamp_seconds",
  "fkst_leader_state",
  "fkst_leader_election_enabled",
  "fkst_leader_ready",
  "fkst_leader_routing_ready",
  "fkst_leader_lease_failures_total",
  "fkst_leader_routing_failures_total",
  "fkst_leader_transitions_total"
]
alert_rules.each do |rule|
  expression = rule.fetch("expr").to_s
  metrics = expression.scan(/\bfkst_[a-z0-9_]+\b/).to_set
  abort "#{rule.fetch('alert')} uses an unreviewed metric" unless (metrics - allowed_metrics).empty?
  unless expression.include?('namespace="chronoai-fkst"') &&
         expression.include?('service="fkst-control-plane-metrics"')
    abort "#{rule.fetch('alert')} lacks the fixed scrape scope"
  end
  abort "#{rule.fetch('alert')} must have a bounded hold duration" if rule["for"].to_s.empty?
  labels = rule.fetch("labels")
  unless labels.keys.to_set == Set["component", "severity"] && labels["component"] == "control-plane"
    abort "#{rule.fetch('alert')} labels are not the fixed reviewed set"
  end
  annotations = rule.fetch("annotations")
  unless annotations.keys.to_set == Set["summary", "description", "runbook_url"]
    abort "#{rule.fetch('alert')} annotations are not the fixed reviewed set"
  end
  if (labels.values + annotations.values).any? { |value| value.to_s.include?("{{") }
    abort "#{rule.fetch('alert')} must not project dynamic labels into notifications"
  end
end

puts "validated optional recovery monitoring and local-only disposable boundary"
