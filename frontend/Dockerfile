# fkst-frontend — the static SPA served by nginx, deployed like every other
# fkst deployable (see opensandbox-developer-guide.md §15). Built once per API
# topology:
# VITE_ vars are baked into the bundle at build time, so a cross-origin
# backend needs a rebuild with VITE_FKST_API_BASE set; the default (empty)
# means SAME-ORIGIN — the normal layout with one ingress fronting both this
# and the fkst-control-plane service.
#
# NOTE: deliberately no `# syntax=` directive — local docker-desktop BuildKit
# hangs on resolving it (see backend/Dockerfile for the same convention).

FROM node:20-alpine AS build
WORKDIR /app
# Lockfile-first for layer caching: dependencies re-install only when the
# lockfile changes, not on every source edit.
COPY package.json package-lock.json ./
RUN npm ci
COPY . .
# Backend base URL baked into the bundle. Empty = same-origin (default
# deployable topology). Set to e.g. https://api.example.com for a
# cross-origin backend (that backend must then allow this origin via CORS).
ARG VITE_FKST_API_BASE=""
ENV VITE_FKST_API_BASE=${VITE_FKST_API_BASE}
RUN npm run build

FROM nginx:1.27-alpine
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /app/dist /usr/share/nginx/html
EXPOSE 80
