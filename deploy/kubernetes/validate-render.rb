#!/usr/bin/env ruby
# frozen_string_literal: true

require "set"
require "yaml"

abort "usage: #{$PROGRAM_NAME} RENDERED_YAML [steady|migration]" unless (1..2).cover?(ARGV.length)

mode = ARGV.fetch(1, "steady")
abort "render mode must be steady or migration" unless %w[steady migration].include?(mode)

documents = YAML.load_stream(File.read(ARGV.fetch(0))).compact
abort "render contains no Kubernetes objects" if documents.empty?

objects = documents.map do |document|
  abort "rendered YAML document is not a mapping" unless document.is_a?(Hash)
  %w[apiVersion kind metadata].each do |field|
    abort "rendered object is missing #{field}" unless document.key?(field)
  end
  metadata = document.fetch("metadata")
  abort "rendered object metadata is not a mapping" unless metadata.is_a?(Hash)
  abort "rendered #{document.fetch('kind')} has no metadata.name" if metadata["name"].to_s.empty?
  document
end

identity = lambda do |object|
  metadata = object.fetch("metadata")
  [object.fetch("kind"), metadata.fetch("namespace", ""), metadata.fetch("name")]
end

duplicates = objects.group_by(&identity).select { |_key, values| values.length > 1 }
unless duplicates.empty?
  abort "render contains duplicate objects: #{duplicates.keys.map { |key| key.join('/') }.join(', ')}"
end

if objects.any? { |object| object.fetch("kind") == "Secret" }
  abort "canonical render must not contain Kubernetes Secret objects"
end

index = objects.to_h { |object| [identity.call(object), object] }
required = [
  ["Namespace", "", "chronoai-fkst"],
  ["ServiceAccount", "chronoai-fkst", "sandbox-runner"],
  ["ServiceAccount", "chronoai-fkst", "fkst-ksa"],
  ["LimitRange", "chronoai-fkst", "sandbox-limits"],
  ["ResourceQuota", "chronoai-fkst", "sandbox-quota"],
  ["NetworkPolicy", "chronoai-fkst", "sandbox-lockdown"],
  ["Role", "chronoai-fkst", "fkst-control-plane-envstore"],
  ["Role", "chronoai-fkst", "fkst-control-plane-leader-election"],
  ["RoleBinding", "chronoai-fkst", "fkst-control-plane-envstore"],
  ["RoleBinding", "chronoai-fkst", "fkst-control-plane-leader-election"],
  ["ConfigMap", "chronoai-fkst", "fkst-control-plane-config"],
  ["ConfigMap", "opensandbox-system", "opensandbox-batchsandbox-template"],
  ["Deployment", "chronoai-fkst", "fkst-control-plane"],
  ["Deployment", "chronoai-fkst", "fkst-frontend"],
  ["Service", "chronoai-fkst", "fkst-control-plane"],
  ["Service", "chronoai-fkst", "fkst-frontend"],
  ["PodDisruptionBudget", "chronoai-fkst", "fkst-control-plane"],
  ["PodDisruptionBudget", "chronoai-fkst", "fkst-frontend"],
  ["Ingress", "chronoai-fkst", "fkst"],
  ["ExternalSecret", "chronoai-fkst", "fkst-control-plane"],
  ["ExternalSecret", "chronoai-fkst", "opensandbox-fkst-api-key"],
  ["ExternalSecret", "chronoai-fkst", "fkst-ingress-tls"],
  ["ExternalSecret", "opensandbox-system", "opensandbox-api-key"]
]
missing = required.reject { |key| index.key?(key) }
abort "render is missing required objects: #{missing.map { |key| key.join('/') }.join(', ')}" unless missing.empty?

config = index.fetch(["ConfigMap", "chronoai-fkst", "fkst-control-plane-config"])
config_data = config["data"] || {}
durable_namespace = config_data["FKST_ENV_STORE_NAMESPACE"].to_s.strip
abort "FKST_ENV_STORE_NAMESPACE must select a durable namespace" if durable_namespace.empty?
abort "durable environment store must be outside chronoai-fkst" if durable_namespace == "chronoai-fkst"
abort "environment-store key material must not be rendered in a ConfigMap" if config_data.key?("FKST_ENV_STORE_ENCRYPTION_KEY") || config_data.key?("FKST_ENV_STORE_ENCRYPTION_KEY_FILE")

durable_required = [
  ["Namespace", "", durable_namespace],
  ["Role", durable_namespace, "fkst-control-plane-durable-envstore"],
  ["RoleBinding", durable_namespace, "fkst-control-plane-durable-envstore"]
]
durable_missing = durable_required.reject { |key| index.key?(key) }
abort "render is missing durable-store objects: #{durable_missing.map { |key| key.join('/') }.join(', ')}" unless durable_missing.empty?

legacy_namespace = config_data["FKST_ENV_STORE_LEGACY_NAMESPACE"].to_s.strip
migration_role_key = ["Role", "chronoai-fkst", "fkst-control-plane-envstore-migration"]
migration_binding_key = ["RoleBinding", "chronoai-fkst", "fkst-control-plane-envstore-migration"]
if mode == "migration"
  abort "migration render must select chronoai-fkst as the legacy namespace" unless legacy_namespace == "chronoai-fkst"
  abort "migration render is missing temporary RBAC" unless index.key?(migration_role_key) && index.key?(migration_binding_key)
else
  abort "steady render must not select a legacy namespace" unless legacy_namespace.empty?
  abort "steady render contains temporary migration RBAC" if index.key?(migration_role_key) || index.key?(migration_binding_key)
end

namespace = index.fetch(["Namespace", "", "chronoai-fkst"])
labels = namespace.dig("metadata", "labels") || {}
abort "namespace must enforce baseline Pod Security" unless labels["pod-security.kubernetes.io/enforce"] == "baseline"
abort "namespace must audit restricted Pod Security" unless labels["pod-security.kubernetes.io/audit"] == "restricted"

durable = index.fetch(["Namespace", "", durable_namespace])
durable_labels = durable.dig("metadata", "labels") || {}
abort "durable namespace must declare the external durability boundary" unless durable_labels["fkst.chronoai.io/durability-boundary"] == "external"

expected_application_rules = [
  {
    "apiGroups" => [""],
    "resources" => ["pods"],
    "verbs" => %w[create get list delete]
  },
  {
    "apiGroups" => [""],
    "resources" => ["pods/log", "pods/status"],
    "verbs" => ["get"]
  }
]
application_role = index.fetch(["Role", "chronoai-fkst", "fkst-control-plane-envstore"])
abort "application env-store Role must contain only validation-Pod permissions" unless application_role["rules"] == expected_application_rules

expected_durable_rules = [
  {
    "apiGroups" => [""],
    "resources" => ["secrets"],
    "verbs" => %w[create get list update delete]
  }
]
durable_role = index.fetch(["Role", durable_namespace, "fkst-control-plane-durable-envstore"])
abort "durable env-store Role must contain exact Secret CRUD permissions" unless durable_role["rules"] == expected_durable_rules

expected_role_ref = {
  "apiGroup" => "rbac.authorization.k8s.io",
  "kind" => "Role",
  "name" => "fkst-control-plane-durable-envstore"
}
expected_subjects = [
  {
    "kind" => "ServiceAccount",
    "name" => "fkst-ksa",
    "namespace" => "chronoai-fkst"
  }
]
durable_binding = index.fetch(["RoleBinding", durable_namespace, "fkst-control-plane-durable-envstore"])
abort "durable env-store RoleBinding roleRef drifted" unless durable_binding["roleRef"] == expected_role_ref
abort "durable env-store RoleBinding subject drifted" unless durable_binding["subjects"] == expected_subjects

if mode == "migration"
  expected_migration_rules = [
    {
      "apiGroups" => [""],
      "resources" => ["configmaps", "secrets"],
      "verbs" => %w[get list delete]
    }
  ]
  migration_role = index.fetch(migration_role_key)
  abort "migration Role must contain exact legacy read/delete permissions" unless migration_role["rules"] == expected_migration_rules

  expected_migration_ref = expected_role_ref.merge("name" => "fkst-control-plane-envstore-migration")
  migration_binding = index.fetch(migration_binding_key)
  abort "migration RoleBinding roleRef drifted" unless migration_binding["roleRef"] == expected_migration_ref
  abort "migration RoleBinding subject drifted" unless migration_binding["subjects"] == expected_subjects
end

sandbox_runner = index.fetch(["ServiceAccount", "chronoai-fkst", "sandbox-runner"])
abort "sandbox-runner must not mount an API token" unless sandbox_runner["automountServiceAccountToken"] == false

network_policy = index.fetch(["NetworkPolicy", "chronoai-fkst", "sandbox-lockdown"])
selector = network_policy.dig("spec", "podSelector", "matchLabels") || {}
abort "sandbox lockdown selector drifted" unless selector["opensandbox.io/workload"] == "sandbox"

objects.select { |object| object.fetch("kind") == "Deployment" }.each do |deployment|
  name = deployment.dig("metadata", "name")
  containers = deployment.dig("spec", "template", "spec", "containers") || []
  abort "#{name} has no container" if containers.empty?
  containers.each do |container|
    image = container["image"].to_s
    abort "#{name} uses an unpinned latest image" if image.end_with?(":latest") || image == "latest"
    abort "#{name} container lacks resources" unless container["resources"].is_a?(Hash)
    abort "#{name} container lacks readinessProbe" unless container["readinessProbe"].is_a?(Hash)
    abort "#{name} container lacks livenessProbe" unless container["livenessProbe"].is_a?(Hash)
  end
end

external_secrets = objects.select { |object| object.fetch("kind") == "ExternalSecret" }
external_secrets.each do |external_secret|
  name = external_secret.dig("metadata", "name")
  store = external_secret.dig("spec", "secretStoreRef") || {}
  abort "#{name} has no external secret store reference" if store["name"].to_s.empty? || store["kind"].to_s.empty?
  target = external_secret.dig("spec", "target") || {}
  abort "#{name} does not retain materialized data" unless target["creationPolicy"] == "Orphan" && target["deletionPolicy"] == "Retain"
  abort "#{name} embeds target secret data" if target.dig("template", "data") || target.dig("template", "stringData")
end

rendered = File.read(ARGV.fetch(0))
secret_signatures = [
  /-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----/,
  /\bgh[opusr]_[A-Za-z0-9]{20,}\b/,
  /\bsk-[A-Za-z0-9]{20,}\b/
]
abort "render appears to contain credential material" if secret_signatures.any? { |pattern| rendered.match?(pattern) }

puts "validated #{objects.length} #{mode} Kubernetes objects (secret values absent)"
