---
"fkst-hosted": minor
---

Scale the fkst-hosted-api deployment to a multi-pod topology (3 replicas, operating range 3–5). Flip the deployment strategy from Recreate to a safe-takeover RollingUpdate (maxUnavailable: 0, maxSurge: 1) now that the distribution layer's MongoDB lease + fencing token serializes per-package engine execution across pods. Wire FKST_POD_ID explicitly from the downward API (metadata.name), add a preStop drain hook, surface the lease/takeover cadence knobs in the ConfigMap, add a PodDisruptionBudget (minAvailable: 2) and a one-line replicas edit-point in kustomization, plus a preferred (soft) pod anti-affinity for multi-node spreading.
