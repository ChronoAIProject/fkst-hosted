---
---

Fix: the worker no longer hard-requires `FKST_POD_ID`. It resolves its id from `FKST_POD_ID` (the Kubernetes downward-API pod name) when set, otherwise falls back to the container `HOSTNAME` (which the runtime sets to the pod name), and only fails closed when neither is available. This lets a worker start in deployments that can only edit the ConfigMap/Secret and cannot wire the downward API into the Pod spec, while keeping each replica's id unique (a static shared id would collide in the controller's registry). The fallback is logged at info. Closes #236.
