# ── Stage 1: Build Rust binary ─────────────────────────────────────────
FROM rust:1.95-bookworm AS rust-builder

RUN apt-get update && \
    apt-get install -y --no-install-recommends cmake && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/

RUN cargo build --release && strip target/release/osm-to-bedrock

# ── Stage 2: Build Next.js frontend ───────────────────────────────────
FROM oven/bun:1 AS web-builder

WORKDIR /build/web
COPY web/package.json web/bun.lock* ./
RUN bun install --frozen-lockfile

COPY web/ .

# RUST_API_URL is read at runtime by the Next.js server (server-side only —
# never inlined into the client bundle), so it is NOT set here. Setting it in
# the build stage would have no effect on the output. Override it at runtime
# via `docker run -e RUST_API_URL=...` (see the runtime stage below).
RUN bun run build

# ── Stage 3: Runtime ──────────────────────────────────────────────────
FROM node:24-bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy Rust binary
COPY --from=rust-builder /build/target/release/osm-to-bedrock /usr/local/bin/osm-to-bedrock

# Copy Next.js standalone output + static assets
COPY --from=web-builder /build/web/.next/standalone ./web/.next/standalone
COPY --from=web-builder /build/web/.next/static ./web/.next/standalone/web/.next/static
COPY --from=web-builder /build/web/public ./web/.next/standalone/web/public

# Copy entrypoint
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

ENV RUST_LOG=info
# Server-side base URL for the Next.js proxy routes → Rust API. Override at
# runtime for remote deploys (`docker run -e RUST_API_URL=https://api.example`).
# Defaulting to localhost keeps `make dev` / local docker-run working.
ENV RUST_API_URL=http://localhost:3002

# SECURITY: the entrypoint binds the API to 0.0.0.0. The Rust binary refuses
# to start without OSM_TO_BEDROCK_API_KEY being set by the operator (or
# OSM_TO_BEDROCK_ALLOW_INSECURE_BIND=1 to explicitly accept the risk of an
# unauthenticated public bind). Do NOT bake a key into this image — operators
# must inject it at runtime via `docker run -e OSM_TO_BEDROCK_API_KEY=...`,
# a Docker secret, or a read-only mounted env file.
EXPOSE 3002 8031

ENTRYPOINT ["docker-entrypoint.sh"]
