# Contributing to osm-to-bedrock

Thank you for your interest in contributing. This document covers how to set up a
development environment, run the checks, and submit a pull request.

## Table of Contents

- [Prerequisites](#prerequisites)
- [Development Setup](#development-setup)
- [Running Checks](#running-checks)
- [Branch and Commit Conventions](#branch-and-commit-conventions)
- [Pull Request Process](#pull-request-process)
- [Code Style](#code-style)
- [Project Layout](#project-layout)

## Prerequisites

| Tool | Notes |
|------|-------|
| Rust stable | Install via `rustup install stable`. The project targets the 2024 edition. |
| cargo | Bundled with Rust. |
| bun 1.1+ | Required only if you are working on the Web Explorer (`web/`). Install from [bun.sh](https://bun.sh). |

## Development Setup

```bash
# Clone the repository
git clone https://github.com/paulrobello/osm-to-bedrock
cd osm-to-bedrock

# Build the Rust binary
make build

# (Optional) install web dependencies if working on the frontend
make web-install

# Start both servers for end-to-end development
make dev
# Rust API → http://localhost:3002
# Web Explorer → http://localhost:8031
```

## Running Checks

**Run all checks before every commit:**

```bash
make checkall
```

This runs `fmt + lint + typecheck + test + web-check` (Rust format/clippy/check/test, then the Next.js frontend's lint + vitest unit tests + production build) and must pass without errors or warnings before any commit is made.

Individual targets:

```bash
make fmt        # rustfmt — auto-formats Rust source
make lint       # cargo clippy --all-targets -- -D warnings
make typecheck  # cargo check
make test       # cargo test
```

For the web frontend:

```bash
cd web
bun run lint    # ESLint
bun run build   # Verify the production build compiles
```

## Branch and Commit Conventions

### Branch names

```
feat/short-description
fix/short-description
docs/short-description
refactor/short-description
```

### Commit message format

```
<type>(<scope>): <subject>

[optional body — what changed and why, 72-char wrapped]

[optional footer — Closes #123]
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`, `perf`

- Subject: max 50 characters, imperative mood, no trailing period
- One logical change per commit

**Examples:**

```
feat(signs): add address signs on building facades

fix(bedrock): correct SubChunk palette index for snow blocks

docs(readme): add fetch-convert subcommand examples
```

## Pull Request Process

1. Fork the repository and create a branch from `main`.
2. Make your changes in small, atomic commits.
3. Run `make checkall` — all checks must pass.
4. Open a pull request against `main` with a clear description of what changed and why.
5. Reference any related issues in the PR description (`Closes #123`).
6. PRs are merged with squash merge; the squash commit message should summarise all changes.

## Code Style

- **Rust**: `rustfmt` for formatting, `clippy` for lints (warnings are errors in CI).
  Follow the existing patterns in each module.
- **TypeScript**: Prettier + ESLint as configured in `web/`. Run `bun run lint` before committing.
- **No `unwrap()` on fallible operations** in production paths — use `?` or handle the error explicitly.
- **No `console.log`** in production TypeScript; gate debug output behind `process.env.NODE_ENV === 'development'`.

## Project Layout

```
src/                  Rust source (CLI + library + Axum server)
├── main.rs           16-LOC binary shim — delegates to cli::main()
├── cli/              clap subcommands (convert/serve/fetch-convert/terrain-convert/overture-convert/cache)
├── pipeline/         streaming tile-based conversion pipeline (mod/render/terrain/preview/decoration/util)
├── server/           Axum HTTP API (mod/state/error/auth/options/handlers)
├── bedrock.rs        BedrockWorld — LevelDB + SubChunk v8 + background ChunkWriter
├── anvil.rs          JavaWorld — Anvil .mca region files (in-memory until streaming lands)
├── world.rs          WorldWriter trait, ChunkStore, Edition enum, ChunkData
├── blocks.rs         Block enum (56 variants) + OSM tag mappings
└── *.rs              geometry/spatial/sign/convert/params/nbt/nbt_be/geojson_export/metadata/config
web/                  Next.js Web Explorer (TypeScript, OpenLayers, React Three Fiber)
docs/                 Project documentation (ARCHITECTURE, CLI, DEVELOPER_INFO, DEPLOYMENT, WEB_UI, …)
Dockerfile            Three-stage build (Rust + bun + node runtime)
docker-entrypoint.sh  Container entrypoint — starts Rust API + Next.js, enforces SEC-001 safe-bind
Makefile              All development targets (build/test/lint/fmt/typecheck/checkall/web-*/docker-*)
Cargo.toml            Rust dependencies and metadata (par-osm-rust pinned to =0.1.1)
```

The project targets both **Bedrock** and **Java Edition** Minecraft output via `--edition bedrock|java`. Bedrock streams tile-by-tile; Java accumulates in memory and is bounded by `enforce_java_memory_budget` until a streaming Anvil writer lands.

See [docs/DEVELOPER_INFO.md](docs/DEVELOPER_INFO.md) for a full architecture reference, [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the streaming tile pipeline and server layout, and [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) for Docker and self-hosting.
