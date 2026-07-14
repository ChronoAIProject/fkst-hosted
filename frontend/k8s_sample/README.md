# fkst-frontend — Kubernetes samples

The frontend deploys like every other fkst deployable: a container image
(`frontend/Dockerfile`, nginx serving the built SPA) plus these sample
manifests. There is no GitHub Pages pipeline.

## Build

```sh
docker build -t fkst-frontend:dev frontend/
```

`VITE_` variables are baked at **build** time. The default build (no args)
targets the **same-origin** topology below — the SPA calls `/api/v1/*` on its
own origin, so no CORS setup exists anywhere. Only a cross-origin backend
needs a rebuild:

```sh
docker build -t fkst-frontend:dev --build-arg VITE_FKST_API_BASE=https://api.example.com frontend/
```

## Deploy

```sh
kubectl apply -n <ns> -k frontend/k8s_sample
```

Same namespace as the backend (`backend/k8s_sample`), one ingress fronting
both: backend public paths (`/api`, `/health`, `/metrics`, `/openapi.json`,
the App webhook) route to `fkst-control-plane`; everything else falls through
to the SPA (client-side routing handles deep links via the nginx fallback).

For the login/dashboard features the backend additionally needs
`FKST_FRONTEND_URL` (where the OAuth callback returns the browser, i.e. this
site's origin), the `FKST_GITHUB_OAUTH_CLIENT_ID/SECRET` pair, and
`FKST_PUBLIC_BASE_URL` — see `backend/k8s_sample/configmap.yaml`. Without
them the docs pages work and login returns 503.
