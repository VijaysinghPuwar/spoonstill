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
# Scratch for `make demo`. Outside the tree: it holds a render, not a fixture.
DEMO_DIR ?= $(CURDIR)/target/demo

.PHONY: help test tts-live lint fmt fixtures brand demo check clean gates gates-m0 gates-m1 gates-m2

help: ## Show available targets
	@grep -hE '^[a-z-]+:.*?## ' $(MAKEFILE_LIST) \
	  | awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-10s\033[0m %s\n",$$1,$$2}'

test: ## Run every test in the workspace
	$(CARGO) test --workspace

tts-live: ## Exercise the Edge provider against the real service (D-094)
	@# Ignored by `make test` on purpose: these cross a network. They prove the
	@# stderr fixtures in edge.rs still match the installed edge-tts, which is
	@# the thing that goes stale.
	$(CARGO) test -p spoonstill-tts --test edge_live -- --ignored --nocapture

lint: ## clippy with warnings denied, plus a formatting check
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(CARGO) fmt --all --check

fmt: ## Reformat the workspace in place
	$(CARGO) fmt --all

brand: ## Regenerate every logo asset from its one description (D-079)
	@python3 scripts/gen-brand.py

demo: ## Rebuild README.md's demo GIF from a real render (D-134)
	@command -v ffmpeg >/dev/null || { echo "needs ffmpeg"; exit 1; }
	python3 scripts/gen-demo.py $(DEMO_DIR)
	cargo build --release -p spoonstill-cli
	./target/release/still render $(DEMO_DIR) --out $(DEMO_DIR)/demo.mp4 \
	  --subtitles boxed --voice en-GB-RyanNeural
	ffmpeg -hide_banner -loglevel error -y -i $(DEMO_DIR)/demo.mp4 \
	  -vf "fps=10,scale=640:-1:flags=lanczos,palettegen=max_colors=96:stats_mode=diff" \
	  $(DEMO_DIR)/palette.png
	ffmpeg -hide_banner -loglevel error -y -i $(DEMO_DIR)/demo.mp4 -i $(DEMO_DIR)/palette.png \
	  -lavfi "fps=10,scale=640:-1:flags=lanczos[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
	  assets/demo/render.gif
	@ls -lh assets/demo/render.gif | awk '{print "  assets/demo/render.gif", $$5}'

fixtures: ## Generate the synthetic test fixtures (see scripts/gen-fixtures.sh)
	@bash scripts/gen-fixtures.sh

check: test lint ## Test and lint — what CI runs

gates: ## Run every milestone's exit gates and report pass/fail
	@bash scripts/m0-gates.sh
	@echo
	@bash scripts/m1-gates.sh
	@echo
	@bash scripts/m2-gates.sh

gates-m0: ## Just the M0 gates
	@bash scripts/m0-gates.sh

gates-m1: ## Just the M1 gates
	@bash scripts/m1-gates.sh

gates-m2: ## Just the M2 gates
	@bash scripts/m2-gates.sh

clean: ## Remove build output and generated fixtures
	$(CARGO) clean
	rm -rf fixtures/generated
