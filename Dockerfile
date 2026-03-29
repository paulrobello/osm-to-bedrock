# ── Stage 1: Build Rust binary ─────────────────────────────────────────
FROM rust:1.87-bookworm AS rust-builder

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

ENV NEXT_PUBLIC_API_URL=http://localhost:3002
RUN bun run build

# ── Stage 3: Runtime ──────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates nodejs && \
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
ENV NEXT_PUBLIC_API_URL=http://localhost:3002

EXPOSE 3002 8031

ENTRYPOINT ["docker-entrypoint.sh"]
