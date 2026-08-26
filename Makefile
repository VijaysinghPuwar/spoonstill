# spoonstill — developer entry points (plan.md M0).
#
# `just` is not installed on this machine and plan.md M0 permits "justfile (or
# Makefile)", so this is make: nothing to install, and CI runs the same targets
# a developer runs.
#
# Homebrew's rustup keeps its shims outside ~/.cargo/bin, so PATH is set here
# too — `make test` works in a shell that has never sourced a rust profile.

export PATH := /opt/homebrew/opt/rustup/bin:$(PATH)

CARGO ?= cargo

.DEFAULT_GOAL := help
.PHONY: help test lint fmt fixtures check clean gates gates-m0 gates-m1

help: ## Show available targets
	@grep -hE '^[a-z-]+:.*?## ' $(MAKEFILE_LIST) \
	  | awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-10s\033[0m %s\n",$$1,$$2}'

test: ## Run every test in the workspace
	$(CARGO) test --workspace

lint: ## clippy with warnings denied, plus a formatting check
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(CARGO) fmt --all --check

fmt: ## Reformat the workspace in place
	$(CARGO) fmt --all

fixtures: ## Generate the synthetic test fixtures (see scripts/gen-fixtures.sh)
	@bash scripts/gen-fixtures.sh

check: test lint ## Test and lint — what CI runs

gates: ## Run every milestone's exit gates and report pass/fail
	@bash scripts/m0-gates.sh
	@echo
	@bash scripts/m1-gates.sh

gates-m0: ## Just the M0 gates
	@bash scripts/m0-gates.sh

gates-m1: ## Just the M1 gates
	@bash scripts/m1-gates.sh

clean: ## Remove build output and generated fixtures
	$(CARGO) clean
	rm -rf fixtures/generated
