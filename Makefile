# One entry point for the checks CI runs.
#
#   make check        CI's go, rust and web jobs, step for step, in the same order
#   make check-fast   only the parts of `check` that the diff from origin/main touches
#   make ci           `check` plus the `CI / container-build` job (needs Docker)
#
# The Apple check (`apple / apple`) needs macOS and Xcode, so it is not here.
# See AGENTS.md and ios/README.md. Keep these recipes in step with
# .github/workflows/ci.yml when that file changes.

SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c
.DEFAULT_GOAL := help

BASE ?= origin/main

.PHONY: help check check-fast ci test fmt lint \
	go-fmt-check go-vet go-test fmt-check rust-test clippy web-deps web-test web-lint \
	extension-lint extension-check extension-browser extension-test \
	check-go check-rust check-web container-check require-ffmpeg

help:
	@echo "make check        run every step of the CI go, rust and web jobs, in CI's order"
	@echo "make check-fast   run only the checks for areas changed since $(BASE)"
	@echo "make ci           check, then the production container build (needs Docker)"
	@echo "make check-go | check-rust | check-web   one CI job"
	@echo "make test         Go, Rust and web unit tests"
	@echo "make lint         gofmt, go vet, rustfmt, clippy, prettier and extension types"
	@echo "make fmt          apply gofmt, rustfmt and prettier"

# Same steps, same order as the go, rust and web jobs in .github/workflows/ci.yml.
check: check-go check-rust check-web

check-go: go-fmt-check go-vet go-test

check-rust: fmt-check rust-test clippy

check-web: web-deps web-test web-lint extension-check extension-test

ci: check container-check

test: go-test rust-test web-test

lint: go-fmt-check go-vet fmt-check clippy web-lint extension-lint

fmt: web-deps
	cd wa-bridge && gofmt -w .
	cargo fmt
	cd web && npm run format

go-fmt-check:
	@cd wa-bridge && unformatted="$$(gofmt -l .)" && if [ -n "$$unformatted" ]; then echo "error: run gofmt -w on these files:"; echo "$$unformatted"; exit 1; fi

go-vet:
	cd wa-bridge && go vet ./...

go-test:
	cd wa-bridge && go test ./...

fmt-check:
	cargo fmt --check

require-ffmpeg:
	@command -v ffmpeg >/dev/null || { echo "ffmpeg is required by the Rust tests (CI installs a static build)."; exit 1; }

# The rust job sets SKIP_GO_BUILD: the go job tests the bridge, and no Rust
# test runs the binary.
rust-test: require-ffmpeg
	SKIP_GO_BUILD=1 cargo test

clippy:
	SKIP_GO_BUILD=1 cargo clippy --all-targets -- -D warnings

# CI runs `npm ci` on a clean checkout. Locally it reruns only when
# package.json or the lock file is newer than the last install, so repeated
# checks stay fast.
web-deps: web/node_modules/.make-npm-ci

web/node_modules/.make-npm-ci: web/package.json web/package-lock.json
	cd web && npm ci
	touch $@

web-test: web-deps
	cd web && npm test

# The web job's lint step also runs lint:extension, via extension-check.
web-lint: web-deps
	cd web && npm run lint:web

extension-lint: web-deps
	cd web && npm run lint:extension && npm run typecheck:extension

# One CI step: prettier, tsc, and the committed chats.html drift check.
extension-check: extension-lint
	cd web && npm run check:extension

# CI runs `npx playwright install chromium`. Locally this downloads the
# browser only when Playwright's pinned build is missing; system libraries
# are left to the machine.
extension-browser: web-deps
	@cd web && if [ -z "$${CHROMIUM_PATH:-}" ] && [ ! -x "$$(node -e 'process.stdout.write(require("playwright").chromium.executablePath())')" ]; then \
		npx playwright install chromium; \
	fi

extension-test: extension-browser
	cd web && CHROMIUM_PATH="$${CHROMIUM_PATH:-$$(node -e 'process.stdout.write(require("playwright").chromium.executablePath())')}" npm run test:extension

container-check:
	docker build --target rust-builder --tag whatsapp-translator:rust-build-check .

# Map changed paths to the checks that cover them, then run those checks in
# CI's order. The Rust tests read web/public and embed the extension's
# chats.html, so web changes also run them. Anything outside the known areas
# (Makefile, CI config, Dockerfile) runs the full `check`.
check-fast:
	@base=$$(git merge-base HEAD $(BASE) 2>/dev/null) || { echo "Cannot find $(BASE); fetch it or pass BASE=<rev>."; exit 1; }; \
	changed=$$(git diff --name-only --no-renames "$$base"); \
	files=$$( { printf '%s\n' "$$changed"; git ls-files --others --exclude-standard; } | sed '/^$$/d' | sort -u ); \
	if [ -z "$$files" ]; then echo "No changes since $(BASE)."; exit 0; fi; \
	go=0; rust=0; web=0; ext=0; full=0; apple=0; \
	while IFS= read -r f; do \
		case "$$f" in \
			wa-bridge/*) go=1 ;; \
			src/*|build.rs|Cargo.toml|Cargo.lock) rust=1 ;; \
			web/extensions/*) ext=1; rust=1 ;; \
			web/package.json|web/package-lock.json) web=1; ext=1 ;; \
			web/*) web=1; rust=1 ;; \
			ios/*) apple=1 ;; \
			*.md|docs/*|assets/*|.claude/*|scripts/generate-message-tones.py|railway.toml|.gitignore|.dockerignore) ;; \
			*) full=1 ;; \
		esac; \
	done <<< "$$files"; \
	if [ "$$apple" = 1 ]; then echo "Apple paths changed: only the apple / apple CI check covers them."; fi; \
	if [ "$$full" = 1 ]; then echo "Shared build or CI files changed: running the full check."; exec $(MAKE) --no-print-directory check; fi; \
	targets=""; \
	[ "$$go" = 1 ] && targets="$$targets go-fmt-check go-vet go-test"; \
	[ "$$rust" = 1 ] && targets="$$targets fmt-check rust-test clippy"; \
	[ "$$web" = 1 ] && targets="$$targets web-test web-lint"; \
	[ "$$ext" = 1 ] && targets="$$targets extension-check extension-test"; \
	if [ -z "$$targets" ]; then echo "Nothing to check for these changes."; exit 0; fi; \
	echo "Running:$$targets"; \
	$(MAKE) --no-print-directory $$targets
