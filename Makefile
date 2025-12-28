.PHONY: help build check test bench fmt lint clean run dev migrate db-reset docker docker-run

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-12s %s\n", $$1, $$2}'

build: ## Build release binary
	cargo build --release

check: ## Type check without building
	cargo check

test: ## Run tests
	cargo test

bench: ## Run benchmarks
	cargo bench

fmt: ## Format code
	cargo fmt

lint: ## Check formatting and run clippy
	cargo fmt --check
	cargo clippy -- -D warnings

clean: ## Remove build artifacts
	cargo clean

run: ## Run release binary
	cargo run --release

dev: ## Run debug binary
	cargo run

migrate: ## Run database migrations
	cargo run --release -- --run-migrations

db-reset: ## Drop and recreate database schema
	psql $(DATABASE_URL) -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"
	cargo run --release -- --run-migrations

docker: ## Build docker image
	docker build -t riskr .

docker-run: ## Run docker container
	docker run -p 8080:8080 riskr
