# Developer entry points. Thin wrappers around cargo, npm,
# pytest and docker compose; `make help` lists them. Nothing here touches a real profile:
# the desktop and the CLI run on .dev/profile, and mail comes from fake servers.

SHELL := /bin/bash
.DEFAULT_GOAL := help

export CARGO_PROFILE_DEV_DEBUG ?= 0

PROFILE_DIR := $(CURDIR)/.dev/profile
VENV := $(CURDIR)/.venv
PY := $(VENV)/bin/python
STACK := docker compose -f deploy/dev/compose.yaml
# `make stack PROFILE=backend` adds the backend to the fake mail servers.
PROFILE ?=
STACK_PROFILE := $(if $(PROFILE),--profile $(PROFILE))
RELEASE_BINS := MAILSYNC_BIN=target/release/mailsync TERN_BIN=target/release/tern TERN_MCP_BIN=target/release/tern-mcp \
	TERN_GATEWAY_BIN=target/release/tern-gateway SAMPLE_SERVICE_BIN=target/release/tern-sample-service

.PHONY: help
help: ## List the targets
	@grep -hE '^[a-z][a-z-]*:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  %-14s %s\n", $$1, $$2}'

.PHONY: setup
setup: check-tools $(VENV) ## Install JS and Python dependencies and build the Rust binaries
	cd apps/desktop && npm ci
	cd apps/web && npm ci
	$(MAKE) build

.PHONY: check-tools
check-tools:
	@command -v cargo >/dev/null || { echo "Rust is missing: https://rustup.rs (rust-toolchain.toml pins the version)"; exit 1; }
	@command -v node >/dev/null || { echo "Node is missing: install the version in .node-version"; exit 1; }
	@node -e 'const want=require("fs").readFileSync(".node-version","utf8").trim(); const have=process.versions.node.split(".")[0]; if (have!==want) { console.error(`Node $${have} found, .node-version wants $${want}`); process.exit(1) }'
	@command -v python3 >/dev/null || { echo "python3 is missing"; exit 1; }
	@command -v docker >/dev/null || echo "docker not found: make stack and the Docker harness tier won't work"

$(VENV): apps/engine/test/requirements.txt
	python3 -m venv $(VENV)
	$(VENV)/bin/pip install -q -r apps/engine/test/requirements.txt
	@touch $(VENV)

.PHONY: build
build: ## Debug builds of mailsync, tern, tern-mcp, tern-gateway, tern-backend and the sample service
	cargo build -p mailsync -p tern -p tern-mcp -p tern-gateway -p tern-backend -p tern-sample-service

.PHONY: release
release: ## Release builds of the same
	cargo build --release --locked -p mailsync -p tern -p tern-mcp -p tern-gateway -p tern-backend \
		-p tern-sample-service

.PHONY: desktop
desktop: build ## Run the desktop on .dev/profile with the debug engine
	ln -sf ../../target/debug/mailsync apps/desktop/mailsync-rs
	ln -sf ../../target/debug/tern apps/desktop/tern
	ln -sf ../../target/debug/tern-mcp apps/desktop/tern-mcp
	mkdir -p $(PROFILE_DIR)
	cd apps/desktop && TERN_ENGINE_KEY_FILE="$(abspath $(PROFILE_DIR))/engine-dev.key" npm start -- --config-dir-path="$(PROFILE_DIR)"

.PHONY: tern
tern: ## Run the CLI against the dev desktop's engine: make tern ARGS="status"
	TERN_CONFIG_DIR="$(PROFILE_DIR)" target/debug/tern $(ARGS)

.PHONY: web
web: ## Vite dev server for apps/web on the mock engine
	cd apps/web && npm run dev

.PHONY: mail
mail: ## Fake mail accounts without Docker (alice and bob @tern.test, password ternpass)
	python3 -I apps/engine/test/tools/dev_mail.py

.PHONY: stack
stack: ## Local stack in Docker: fake mail, and the backend with PROFILE=backend
	$(STACK) $(STACK_PROFILE) up -d --build --wait
	@if [ "$(PROFILE)" = backend ] && ! $(STACK) exec -T backend tern-backend user list | grep -q alice@tern.test; then \
	  echo ternpass | $(STACK) exec -T backend tern-backend user add alice@tern.test --first-name Alice --password-stdin; \
	fi
	@$(STACK) $(STACK_PROFILE) logs mail | grep -E 'IMAP .* SMTP' | sed 's/^.*| //'
	@if [ "$(PROFILE)" = backend ]; then echo "backend: http://127.0.0.1:5101, user alice@tern.test, password ternpass"; fi

.PHONY: stack-down
stack-down: ## Stop the local stack, keeping its data
	$(STACK) --profile backend down

.PHONY: stack-reset
stack-reset: ## Stop the local stack and delete its data
	$(STACK) --profile backend down -v

.PHONY: test
test: $(VENV) ## Workspace tests and the harness quick tier (debug engine)
	cargo test --workspace
	cargo build -p mailsync -p tern -p tern-mcp -p tern-gateway -p tern-sample-service
	MAILSYNC_BIN=target/debug/mailsync TERN_BIN=target/debug/tern TERN_MCP_BIN=target/debug/tern-mcp \
		TERN_GATEWAY_BIN=target/debug/tern-gateway SAMPLE_SERVICE_BIN=target/debug/tern-sample-service $(PY) -m pytest apps/engine/test -q

# Leaves an existing apps/desktop/mailsync-rs alone (a release engine from build-release.sh or
# trial-desktop.sh); links the debug engine only when there is none.
.PHONY: test-desktop
test-desktop: ## Desktop Jasmine specs: make test-desktop SPEC=calendar runs the matching spec files
	@[ -e apps/desktop/mailsync-rs ] || { cargo build -p mailsync && ln -sf ../../target/debug/mailsync apps/desktop/mailsync-rs; }
	cd apps/desktop && npm test $(if $(SPEC),-- --spec-file-pattern='$(SPEC)')

.PHONY: lint
lint: ## fmt, clippy, cargo deny, and the JS linters
	cargo fmt --all --check
	cargo clippy --workspace --all-targets --locked -- -D warnings
	cargo deny check
	cd apps/desktop && npm run lint:check
	cd apps/web && npm run lint && npm run typecheck

.PHONY: gate
gate: $(VENV) ## The merge gate: lint, tests, release builds, harness quick tier on serve
	cargo fmt --all --check
	cargo clippy --workspace --all-targets --locked -- -D warnings
	cargo test --workspace --locked
	cargo deny check
	$(MAKE) release
	MAILSYNC_TRANSPORT=serve REQUIRE_TERN=1 $(RELEASE_BINS) $(PY) -m pytest apps/engine/test -q
