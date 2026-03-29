#!/bin/sh
set -e

# Start the Rust API server in the background
echo "Starting Rust API server on port ${API_PORT:-3002}..."
osm-to-bedrock serve --host 0.0.0.0 --port "${API_PORT:-3002}" &
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
