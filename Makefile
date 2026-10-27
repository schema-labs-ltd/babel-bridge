# One entry point for the checks CI runs. Nx runs every task (see nx.json and
# each project's project.json) and caches results in .nx/, so a rerun with
# unchanged inputs replays the stored output instead of running again.
#
#   make check        CI's go, rust and web jobs: every project's checks
#   make check-fast   only the checks for projects changed since origin/main (nx affected)
#   make ci           `check` plus the `CI / container-build` job (needs Docker)
#
# The Apple check (`apple / apple`) needs macOS and Xcode, so it is not here.
# See AGENTS.md and ios/README.md. Keep the project.json targets in step with
# .github/workflows/ci.yml when that file changes.

SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c
.DEFAULT_GOAL := help

BASE ?= origin/main
NX := npx --no-install nx

# The targets `check` runs. ios:api-contract needs a Swift toolchain, so it
# runs in its own CI job and through `make api-contract`.
CHECK_TARGETS := format-check lint typecheck test check e2e
CHECK_FLAGS := --exclude=ios --outputStyle=stream

.PHONY: help check check-fast ci test fmt lint nx-deps \
	go-fmt-check go-vet go-test fmt-check rust-test clippy web-deps web-test web-lint \
	extension-lint extension-check extension-browser extension-test \
	check-go check-rust check-web container-check require-ffmpeg scripts-test scripts-lint \
	api-contract graph

help:
	@echo "make check        run every project's checks through Nx (CI's go, rust and web jobs)"
	@echo "make check-fast   run only the checks for projects changed since $(BASE) (nx affected)"
	@echo "make ci           check, then the production container build (needs Docker)"
	@echo "make check-go | check-rust | check-web   one CI job"
	@echo "make test         Go, Rust, web and tools/scripts unit tests"
	@echo "make lint         gofmt, go vet, rustfmt, clippy, prettier (web, extension, tools/scripts) and extension types"
	@echo "make fmt          apply gofmt, rustfmt and prettier"
	@echo "make graph        open the Nx project graph"
	@echo "Any task also runs directly: npx nx run <project>:<target>, e.g. npx nx test server"

check: web-deps extension-browser require-ffmpeg
	$(NX) run-many -t $(CHECK_TARGETS) $(CHECK_FLAGS)

check-go: nx-deps
	$(NX) run-many -t format-check lint test -p wa-bridge --outputStyle=stream

check-rust: nx-deps require-ffmpeg
	$(NX) run-many -t format-check test lint -p server --outputStyle=stream

check-web: web-deps extension-browser
	$(NX) run-many -t $(CHECK_TARGETS) -p web web-extension tools-scripts --outputStyle=stream

ci: check container-check

test: web-deps require-ffmpeg
	$(NX) run-many -t test $(CHECK_FLAGS)

lint: web-deps
	$(NX) run-many -t format-check lint typecheck $(CHECK_FLAGS)

fmt: web-deps
	$(NX) run-many -t format $(CHECK_FLAGS)

graph: nx-deps
	$(NX) graph

go-fmt-check: nx-deps
	$(NX) run wa-bridge:format-check

go-vet: nx-deps
	$(NX) run wa-bridge:lint

go-test: nx-deps
	$(NX) run wa-bridge:test

fmt-check: nx-deps
	$(NX) run server:format-check

require-ffmpeg:
	@command -v ffmpeg >/dev/null || { echo "ffmpeg is required by the Rust tests (CI installs a static build)."; exit 1; }

# server:test and server:lint set SKIP_GO_BUILD: wa-bridge:test covers the
# bridge, and no Rust test runs the binary.
rust-test: nx-deps require-ffmpeg
	$(NX) run server:test

clippy: nx-deps
	$(NX) run server:lint

# Nx and every npm workspace (apps/web, libs/web-extension) install from the
# root lock file. CI runs `npm ci` on a clean checkout; locally it reruns only
# when a manifest or the lock file is newer than the last install.
nx-deps: node_modules/.make-npm-ci

node_modules/.make-npm-ci: package.json package-lock.json apps/web/package.json libs/web-extension/package.json
	npm ci
	touch $@

# Kept as an alias: the web app and extension share the root install.
web-deps: nx-deps

web-test: web-deps
	$(NX) run web:test

# The risk tier and merge health scripts in tools/scripts (CI's web job).
scripts-test: nx-deps
	$(NX) run tools-scripts:test

scripts-lint: web-deps
	$(NX) run tools-scripts:lint

web-lint: web-deps
	$(NX) run web:lint

extension-lint: web-deps
	$(NX) run-many -t lint typecheck -p web-extension

# prettier, tsc, and the committed chats.html drift check.
extension-check: web-deps
	$(NX) run web-extension:check

# CI runs `npx playwright install chromium`. Locally this downloads the
# browser only when Playwright's pinned build is missing; system libraries
# are left to the machine.
extension-browser: web-deps
	@if [ -z "$${CHROMIUM_PATH:-}" ] && [ ! -x "$$(node -e 'process.stdout.write(require("playwright").chromium.executablePath())')" ]; then \
		npx playwright install chromium; \
	fi

extension-test: extension-browser
	$(NX) run web-extension:e2e

api-contract: nx-deps
	$(NX) run ios:api-contract

container-check: nx-deps
	$(NX) run server:container-check

# Nx maps changed files to projects (project roots and nx.json namedInputs). A change
# to nx.json, the root lock file, this Makefile or ci.yml affects every
# project; docs and other files outside apps/, libs/ and tools/ affect none.
check-fast: web-deps
	@git merge-base HEAD $(BASE) >/dev/null 2>&1 || { echo "Cannot find $(BASE); fetch it or pass BASE=<rev>."; exit 1; }
	@if { git diff --name-only $(BASE)...; git diff --name-only HEAD; git ls-files --others --exclude-standard; } | grep '^apps/ios/' >/dev/null; then echo "Apple paths changed: only the apple / apple CI check covers them."; fi
	$(NX) affected -t $(CHECK_TARGETS) --base=$(BASE) $(CHECK_FLAGS)
