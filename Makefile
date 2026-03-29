.PHONY: build test lint fmt typecheck checkall web-check clean install web-dev web-build web-install web-kill kill stop serve-stop web-stop docker-build docker-run docker-stop

build:
	cargo build --release

test:
	cargo test

lint:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt

typecheck:
	cargo check

web-check: ## Lint and build-check the Next.js frontend
	cd web && bun run lint && bun run build

checkall: fmt lint typecheck test web-check

clean:
	cargo clean

install:
	cargo install --path .

web-dev:
	cd web && bun run dev

web-build:
	cd web && bun run build

web-install:
	cd web && bun install

serve: ## Start Rust API server on port 3002
	cargo run --release -- serve

dev: ## Start both Rust API + Next.js dev servers
	@echo "Starting Rust API server..."
	@cargo run --release -- serve &
	@echo "Starting Next.js dev server..."
	@cd web && bun run dev

serve-stop: ## Gracefully stop the Rust API server on port 3002
	@lsof -ti:3002 | xargs kill 2>/dev/null || echo "No server running on port 3002"

web-stop: ## Gracefully stop the Next.js dev server on port 8031
	@lsof -ti:8031 | xargs kill 2>/dev/null || echo "No server running on port 8031"

stop: ## Gracefully stop both servers (ports 3002 + 8031)
	@lsof -ti:3002 | xargs kill 2>/dev/null || true
	@lsof -ti:8031 | xargs kill 2>/dev/null || true
	@echo "Stopped all dev servers"

web-kill: ## Force-kill Next.js dev server on port 8031
	@lsof -ti:8031 | xargs kill -9 2>/dev/null || true

kill: ## Force-kill both dev servers (ports 3002 + 8031)
	@lsof -ti:3002 | xargs kill -9 2>/dev/null || true
	@lsof -ti:8031 | xargs kill -9 2>/dev/null || true

# Example: convert a PBF file
# make convert INPUT=city.osm.pbf OUTPUT=~/games/minecraft/worlds/MyCity
convert:
	cargo run --release -- convert --input $(INPUT) --output $(OUTPUT)

docker-build: ## Build the Docker image
	docker build -t osm-to-bedrock .

docker-run: ## Run the Docker container (API on 3002, web on 8031)
	docker run --rm -p 3002:3002 -p 8031:8031 --name osm-to-bedrock osm-to-bedrock

docker-stop: ## Stop the running Docker container
	docker stop osm-to-bedrock 2>/dev/null || echo "No container running"
