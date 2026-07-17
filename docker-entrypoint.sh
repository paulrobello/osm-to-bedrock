#!/bin/sh
set -e

# Start the Rust API server in the background.
#
# SECURITY: this image binds the API to 0.0.0.0, which is NOT a loopback
# address. The Rust binary refuses to start in that case unless EITHER:
#   - an API key is provided via $OSM_TO_BEDROCK_API_KEY (recommmended —
#     enables shared-secret auth on all mutating routes plus /download,
#     /status, /cache), OR
#   - $OSM_TO_BEDROCK_ALLOW_INSECURE_BIND=1 is set (explicitly acknowledges
#     the risk of running unauthenticated on a public interface).
#
# Never hardcode a real API key in this file or in the Dockerfile. Operators
# must inject it at runtime via `docker run -e OSM_TO_BEDROCK_API_KEY=...`,
# a Docker secret, or a `.env` file mounted read-only.
echo "Starting Rust API server on port ${API_PORT:-3002}..."
osm-to-bedrock serve --host 0.0.0.0 --port "${API_PORT:-3002}" ${API_KEY_FLAG:-} &
RUST_PID=$!

# Start the Next.js standalone server
echo "Starting web UI on port ${PORT:-8031}..."
cd /app/web
PORT="${PORT:-8031}" HOSTNAME="0.0.0.0" node .next/standalone/web/server.js &
NODE_PID=$!

# Graceful shutdown
cleanup() {
    echo "Shutting down..."
    kill "$NODE_PID" 2>/dev/null || true
    kill "$RUST_PID" 2>/dev/null || true
    wait "$NODE_PID" 2>/dev/null || true
    wait "$RUST_PID" 2>/dev/null || true
    exit 0
}
trap cleanup SIGTERM SIGINT

# Wait for either process to exit
wait -n "$RUST_PID" "$NODE_PID" 2>/dev/null || true
cleanup
