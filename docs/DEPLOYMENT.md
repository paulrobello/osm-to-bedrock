# Deployment

How to run osm-to-bedrock beyond a single developer laptop — via Docker, as a self-hosted Rust API + Next.js frontend pair, or behind a reverse proxy. Covers the ports, environment variables, and the **authentication requirement** that applies as soon as you bind a non-loopback interface.

## Table of Contents

- [Quick Start with Docker](#quick-start-with-docker)
- [Environment Variables](#environment-variables)
- [Authentication Requirement for Non-Loopback Binds](#authentication-requirement-for-non-loopback-binds)
- [Self-Hosted Without Docker](#self-hosted-without-docker)
- [Running Behind a Reverse Proxy](#running-behind-a-reverse-proxy)
- [Related Documentation](#related-documentation)

## Quick Start with Docker

The repository ships a three-stage `Dockerfile` (Rust build → Next.js build → `node:24-bookworm-slim` runtime) and a `docker-entrypoint.sh` that starts both processes.

```bash
# Build the image
make docker-build
# Or: docker build -t osm-to-bedrock .

# Run it (binds both ports; requires an API key — see below)
docker run --rm \
  -p 3002:3002 -p 8031:8031 \
  -e OSM_TO_BEDROCK_API_KEY="$(openssl rand -hex 32)" \
  --name osm-to-bedrock osm-to-bedrock

# Stop and remove
make docker-stop
# Or: docker stop osm-to-bedrock
```

Once running:

- Rust API on `http://<host>:3002` (`/health` is public; all other routes require the API key)
- Web Explorer on `http://<host>:8031`

The Makefile wraps the common commands:

| Target | Action |
|--------|--------|
| `make docker-build` | `docker build -t osm-to-bedrock .` |
| `make docker-run` | `docker run --rm -p 3002:3002 -p 8031:8031 --name osm-to-bedrock osm-to-bedrock` |
| `make docker-stop` | `docker stop osm-to-bedrock` (no-op if not running) |

> **Note:** `make docker-run` in the Makefile does **not** inject an API key. It works for local testing because the container's entrypoint binds to `0.0.0.0`, which the Rust binary refuses without a key. For local Docker, set `OSM_TO_BEDROCK_ALLOW_INSECURE_BIND=1` to acknowledge the risk, or extend the target to inject a key.

## Environment Variables

| Variable | Default | Where read | Description |
|----------|---------|-----------|-------------|
| `OSM_TO_BEDROCK_API_KEY` | unset | Rust API (`serve`) | Shared-secret API key. When set, all routes except `/health` require it in the `Authorization: Bearer <key>` (or `X-API-Key: <key>`) header. Set this for any non-loopback deployment. |
| `OSM_TO_BEDROCK_ALLOW_INSECURE_BIND` | unset | Rust API (`serve`) | Set to `1` to explicitly accept the risk of binding a non-loopback interface without an API key. Avoid in production. |
| `CORS_ALLOWED_ORIGIN` | `http://localhost:8031` | Rust API (`serve`) | Origin allowed by the API's CORS layer. Set to the Web Explorer's public origin (e.g. `https://maps.example.com`) so browser requests from the frontend are accepted. |
| `RUST_API_URL` | `http://localhost:3002` | Next.js proxy routes (server-side) | Base URL the Next.js `/api/*` routes use to reach the Rust API. Override when the API runs on a different host/port. **Not** `NEXT_PUBLIC_`-prefixed — never inlined into the browser bundle, so it can be changed at runtime without a rebuild. |
| `RUST_LOG` | `info` | Rust API | Log verbosity (`error`, `warn`, `info`, `debug`, `trace`). |
| `OVERPASS_URL` | `https://overpass-api.de/api/interpreter` | Rust API + CLI | Override the Overpass API endpoint (useful for mirrors). |
| `OVERPASS_CACHE_DIR` | `~/.cache/osm-to-bedrock/overpass/` | Rust API + CLI | Override the disk cache directory. |
| `API_PORT` | `3002` | `docker-entrypoint.sh` | Port the Rust API listens on inside the container. |
| `PORT` | `8031` | `docker-entrypoint.sh` / Next.js | Port the Next.js standalone server listens on inside the container. |

## Authentication Requirement for Non-Loopback Binds

The Rust API server refuses to start when `--host` (or its default) is a non-loopback address and no API key is configured, unless `OSM_TO_BEDROCK_ALLOW_INSECURE_BIND=1` is also set. This is intentional: the API accepts multi-hundred-MB uploads and has no per-user rate limiting, so an open bind is a DoS and resource-exhaustion risk.

**Always set `OSM_TO_BEDROCK_API_KEY` when exposing the API beyond loopback** — including in the Docker container, whose entrypoint binds `--host 0.0.0.0`. Inject the key at runtime; do not bake it into the image.

```bash
# Generate a strong key
openssl rand -hex 32

# Inject at runtime via -e (or a Docker secret / mounted env file)
docker run --rm \
  -p 3002:3002 -p 8031:8031 \
  -e OSM_TO_BEDROCK_API_KEY="<your-key-here>" \
  osm-to-bedrock
```

Clients then send the key in every request to a protected route:

```bash
curl -H "Authorization: Bearer <your-key-here>" http://host:3002/cache/areas
# Equivalently:
curl -H "X-API-Key: <your-key-here>" http://host:3002/cache/areas
```

The Next.js Web Explorer does not yet forward an API key to the Rust API, so a key-protected API is currently only drivable via direct HTTP (CLI, curl, scripts). Running the Web Explorer and the Rust API behind the same auth boundary (reverse proxy or private network) is the practical workaround until the frontend learns to send the key.

## Self-Hosted Without Docker

If you already have a Rust + Node runtime:

```bash
# Build the Rust binary
make build

# Build the Next.js frontend (standalone output)
make web-build

# Start the Rust API on 127.0.0.1:3002 (loopback; no key required)
make serve
# Or for production with a key:
OSM_TO_BEDROCK_API_KEY="<key>" \
  ./target/release/osm-to-bedrock serve --host 0.0.0.0 --port 3002

# In another shell, start the Next.js frontend
cd web
RUST_API_URL=http://localhost:3002 \
  PORT=8031 HOSTNAME=0.0.0.0 \
  node .next/standalone/web/server.js
```

For non-loopback binds, set `OSM_TO_BEDROCK_API_KEY` (and optionally `CORS_ALLOWED_ORIGIN` to match your frontend's public origin).

## Running Behind a Reverse Proxy

A typical production setup places both services behind a reverse proxy (Caddy, Traefik, Nginx, Cloudflare Tunnel, etc.) that terminates TLS and forwards to the loopback ports:

```text
Browser ──HTTPS──▶ Reverse Proxy
                    ├─ /      → 127.0.0.1:8031  (Next.js)
                    └─ /api/* → 127.0.0.1:3002  (Rust API, optional auth)
```

When the reverse proxy keeps both services on the same origin (e.g. `https://maps.example.com/` for the frontend and `https://maps.example.com/api/*` rewritten to `/` on the Rust API), set:

- `CORS_ALLOWED_ORIGIN=https://maps.example.com` (or unset if everything is same-origin)
- `RUST_API_URL=http://127.0.0.1:3002` (the proxy reaches the API over loopback)

Keep the Rust API bound to `127.0.0.1` so the proxy is the only exposure path; the safe-bind guard will not require an API key in that case. If you still want to require an API key for browser-driven requests (defence in depth), remember the Web Explorer does not yet send the key — gate the API at the proxy layer instead.

## Related Documentation

- [CLI Reference](CLI.md) — `serve` subcommand flags and environment variables
- [Architecture](ARCHITECTURE.md) — server module layout and API endpoints
- [Web Explorer](WEB_UI.md) — Next.js frontend and proxy routes
