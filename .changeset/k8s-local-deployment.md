---
"fkst-hosted": minor
---

Add Kubernetes manifests for the local single-replica deployment: namespace, ConfigMap, secret template, MongoDB StatefulSet with a persistent volume and headless Service, the fkst-hosted-api Deployment (Recreate strategy, hardened security context, health probes, writable engine runtime volume) with its ClusterIP Service, and a kustomization tying it together with an image-tag seam.
