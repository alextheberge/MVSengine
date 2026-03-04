SHELL := /usr/bin/env bash
.SHELLFLAGS := -eu -o pipefail -c

MAKEFLAGS += --warn-undefined-variables
MAKEFLAGS += --no-builtin-rules

.DEFAULT_GOAL := help

PROJECT_ROOT := $(abspath .)
CARGO ?= cargo
BIN ?= mvs-manager

ROOT ?= .
MANIFEST ?= mvs.json
CONTEXT ?= cli
AI_SCHEMA ?=
ARCH_BREAK ?= false
ARCH_REASON ?=
HOST_MANIFEST ?= tests/fixtures/manifests/host_with_shim.json
EXTENSION_MANIFEST ?= tests/fixtures/manifests/extension_out_of_range.json
ALLOW_SHIMS ?= true
TARGET ?=

VERSION_TAG ?= v$(shell $(CARGO) metadata --no-deps --format-version=1 2>/dev/null | sed -n 's/.*"version":"\([^"]*\)".*/\1/p' | head -n1)
DIST_ROOT ?= dist
RELEASE_TARGETS ?= x86_64-unknown-linux-gnu x86_64-apple-darwin aarch64-apple-darwin x86_64-pc-windows-msvc
ARCHIVE_PATH ?=
CHECKSUMS_PATH ?=
SIGNATURE_PATH ?=
PUBLIC_KEY_PATH ?=
GPG_PRIVATE_KEY_FILE ?=
INSTALL_REPO ?= alextheberge/MVSengine
INSTALL_VERSION ?= latest
INSTALL_DIR ?= $(HOME)/.local/bin

HOST_TARGET := $(shell rustc -vV 2>/dev/null | awk '/host:/ {print $$2}')
CARGO_TARGET_FLAG := $(if $(strip $(TARGET)),--target $(TARGET),)

.PHONY: help print-config bootstrap fmt fmt-check check clippy test test-unit test-integration build build-release docs clean ci generate generate-dry lint-manifest validate fixture-smoke release-local release-host release-target release-matrix-local release-merge-checksums release-sign-checksums release-verify install watch doctor

help: ## Show all available targets.
	@awk 'BEGIN {FS = ":.*##"; printf "\nMVS Engine Make Targets\n\n"} /^[a-zA-Z0-9_.-]+:.*##/ { printf "  %-26s %s\n", $$1, $$2 }' $(MAKEFILE_LIST)

print-config: ## Print effective build/runtime configuration.
	@printf "PROJECT_ROOT=%s\n" "$(PROJECT_ROOT)"
	@printf "CARGO=%s\n" "$(CARGO)"
	@printf "BIN=%s\n" "$(BIN)"
	@printf "ROOT=%s\n" "$(ROOT)"
	@printf "MANIFEST=%s\n" "$(MANIFEST)"
	@printf "CONTEXT=%s\n" "$(CONTEXT)"
	@printf "AI_SCHEMA=%s\n" "$(AI_SCHEMA)"
	@printf "TARGET=%s\n" "$(TARGET)"
	@printf "VERSION_TAG=%s\n" "$(VERSION_TAG)"
	@printf "DIST_ROOT=%s\n" "$(DIST_ROOT)"
	@printf "HOST_TARGET=%s\n" "$(HOST_TARGET)"

bootstrap: ## Pre-fetch Rust dependencies.
	$(CARGO) fetch

fmt: ## Format Rust code.
	$(CARGO) fmt

fmt-check: ## Verify formatting.
	$(CARGO) fmt -- --check

check: ## Type-check all targets.
	$(CARGO) check --all-targets

clippy: ## Run strict clippy lints.
	$(CARGO) clippy --all-targets --all-features -- -D warnings

test: ## Run all unit + integration tests.
	$(CARGO) test --all-targets

test-unit: ## Run library/unit tests only.
	$(CARGO) test --lib

test-integration: ## Run integration CLI tests.
	$(CARGO) test --test integration_cli

build: ## Build debug binary.
	$(CARGO) build $(CARGO_TARGET_FLAG)

build-release: ## Build release binary (optionally pass TARGET=<triple>).
	$(CARGO) build --release $(CARGO_TARGET_FLAG)

docs: ## Build Rust API docs for local inspection.
	$(CARGO) doc --no-deps

generate: ## Run manifest generator (writes MANIFEST).
	@args=(generate --root "$(ROOT)" --manifest "$(MANIFEST)" --context "$(CONTEXT)"); \
	if [[ "$(ARCH_BREAK)" == "true" ]]; then args+=(--arch-break); fi; \
	if [[ -n "$(ARCH_REASON)" ]]; then args+=(--arch-reason "$(ARCH_REASON)"); fi; \
	if [[ -n "$(AI_SCHEMA)" ]]; then args+=(--ai-schema "$(AI_SCHEMA)"); fi; \
	$(CARGO) run -- "$${args[@]}"

generate-dry: ## Run generator in dry-run mode.
	@args=(generate --root "$(ROOT)" --manifest "$(MANIFEST)" --context "$(CONTEXT)" --dry-run); \
	if [[ "$(ARCH_BREAK)" == "true" ]]; then args+=(--arch-break); fi; \
	if [[ -n "$(ARCH_REASON)" ]]; then args+=(--arch-reason "$(ARCH_REASON)"); fi; \
	if [[ -n "$(AI_SCHEMA)" ]]; then args+=(--ai-schema "$(AI_SCHEMA)"); fi; \
	$(CARGO) run -- "$${args[@]}"

lint-manifest: ## Verify code and manifest are aligned.
	@args=(lint --root "$(ROOT)" --manifest "$(MANIFEST)"); \
	if [[ -n "$(AI_SCHEMA)" ]]; then args+=(--ai-schema "$(AI_SCHEMA)"); fi; \
	$(CARGO) run -- "$${args[@]}"

validate: ## Validate extension compatibility against host manifest.
	$(CARGO) run -- validate \
		--host-manifest "$(HOST_MANIFEST)" \
		--extension-manifest "$(EXTENSION_MANIFEST)" \
		--allow-shims "$(ALLOW_SHIMS)"

fixture-smoke: ## Run generator/linter against fixture project.
	@tmp_dir="$$(mktemp -d)"; \
	trap 'rm -rf "$$tmp_dir"' EXIT; \
	cp -R tests/fixtures/generator_project "$$tmp_dir/project"; \
	manifest_path="$$tmp_dir/mvs.json"; \
	$(CARGO) run -- generate --root "$$tmp_dir/project" --manifest "$$manifest_path" --context cli --ai-schema "$$tmp_dir/project/tool_schema.json"; \
	$(CARGO) run -- lint --root "$$tmp_dir/project" --manifest "$$manifest_path" --ai-schema "$$tmp_dir/project/tool_schema.json"

release-host: ## Build/package release artifact for the current host target.
	@TARGET="$(HOST_TARGET)" VERSION_TAG="$(VERSION_TAG)" DIST_ROOT="$(DIST_ROOT)" scripts/release/package.sh

release-target: ## Build/package release artifact for TARGET=<triple>.
	@if [[ -z "$(TARGET)" ]]; then \
		echo "Set TARGET=<triple>, for example TARGET=x86_64-unknown-linux-gnu"; \
		exit 1; \
	fi
	@TARGET="$(TARGET)" VERSION_TAG="$(VERSION_TAG)" DIST_ROOT="$(DIST_ROOT)" scripts/release/package.sh

release-matrix-local: ## Attempt local packaging for RELEASE_TARGETS (requires cross toolchains).
	@for target in $(RELEASE_TARGETS); do \
		echo "Packaging $$target"; \
		TARGET="$$target" VERSION_TAG="$(VERSION_TAG)" DIST_ROOT="$(DIST_ROOT)" scripts/release/package.sh; \
	done

release-merge-checksums: ## Merge *.sha256 files into dist/<VERSION_TAG>/checksums.txt.
	scripts/release/merge_checksums.sh "$(DIST_ROOT)/$(VERSION_TAG)"

release-sign-checksums: ## Sign dist/<VERSION_TAG>/checksums.txt using GPG_PRIVATE_KEY_FILE=<path>.
	@if [[ -z "$(GPG_PRIVATE_KEY_FILE)" ]]; then \
		echo "Set GPG_PRIVATE_KEY_FILE=<path/to/private.key.asc>"; \
		exit 1; \
	fi
	@if ! command -v gpg >/dev/null 2>&1; then \
		echo "gpg is required for signing"; \
		exit 1; \
	fi
	@gpg --batch --import "$(GPG_PRIVATE_KEY_FILE)" >/dev/null 2>&1
	@gpg --batch --yes --armor --detach-sign --output "$(DIST_ROOT)/$(VERSION_TAG)/checksums.txt.asc" "$(DIST_ROOT)/$(VERSION_TAG)/checksums.txt"
	@echo "Signed checksums file at $(DIST_ROOT)/$(VERSION_TAG)/checksums.txt.asc"

release-verify: ## Verify ARCHIVE_PATH against CHECKSUMS_PATH and optional signature/public key.
	@if [[ -z "$(ARCHIVE_PATH)" || -z "$(CHECKSUMS_PATH)" ]]; then \
		echo "Set ARCHIVE_PATH=<archive> and CHECKSUMS_PATH=<checksums.txt>"; \
		exit 1; \
	fi
	@args=("$(ARCHIVE_PATH)" "$(CHECKSUMS_PATH)"); \
	if [[ -n "$(SIGNATURE_PATH)" || -n "$(PUBLIC_KEY_PATH)" ]]; then \
		args+=("$(SIGNATURE_PATH)" "$(PUBLIC_KEY_PATH)"); \
	fi; \
	scripts/verify-release.sh "$${args[@]}"

release-local: release-host ## Alias for host release packaging.
	@echo "Release artifacts under $(DIST_ROOT)/$(VERSION_TAG)"

install: ## Install released binary via scripts/install.sh.
	@MVS_REPO="$(INSTALL_REPO)" MVS_VERSION="$(INSTALL_VERSION)" MVS_INSTALL_DIR="$(INSTALL_DIR)" scripts/install.sh

ci: fmt-check check clippy test fixture-smoke lint-manifest ## Full local/CI quality gate.
	@echo "CI checks passed."

watch: ## Run cargo watch loop if available.
	@if command -v cargo-watch >/dev/null 2>&1; then \
		cargo watch -x check -x test; \
	else \
		echo "cargo-watch is not installed. Run: cargo install cargo-watch"; \
		exit 1; \
	fi

doctor: ## Verify required developer tools are available.
	@for tool in $(CARGO) make; do \
		if ! command -v $$tool >/dev/null 2>&1; then \
			echo "missing required tool: $$tool"; \
			exit 1; \
		fi; \
	done; \
	echo "tooling looks good"

clean: ## Remove target artifacts.
	$(CARGO) clean
