#!/usr/bin/env ruby
# frozen_string_literal: true

require "set"
require "yaml"

abort "usage: #{$PROGRAM_NAME} RENDERED_YAML" unless ARGV.length == 1

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

namespace = index.fetch(["Namespace", "", "chronoai-fkst"])
labels = namespace.dig("metadata", "labels") || {}
abort "namespace must enforce baseline Pod Security" unless labels["pod-security.kubernetes.io/enforce"] == "baseline"
abort "namespace must audit restricted Pod Security" unless labels["pod-security.kubernetes.io/audit"] == "restricted"

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

puts "validated #{objects.length} rendered Kubernetes objects (secret values absent)"
