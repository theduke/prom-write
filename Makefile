
default: ci

# Actions

fmt:
	@echo "Formatting rust code with cargo fmt..."
	cargo fmt --all
	@echo "Code formatted!"
	@echo ""

clippyfix:
	@echo "Fixing clippy lints..."
	cargo clippy --fix -- -D warnings
	@echo ""

# Run a local Prometheus server for testing.
run-prometheus-docker:
	docker run --rm -p 9090:9090 prom/prometheus \
	--config.file=/etc/prometheus/prometheus.yml \
	--web.enable-remote-write-receiver

changelog:
	@echo "Generating changelog..."
	git-cliff --unreleased --prepend CHANGELOG.md
	@echo "Changelog generated!"
	@echo ""

fix: clippyfix fmt
	@echo "All fixes applied!"
	@echo ""

# Linting

check-fmt:
	@echo Checking formatting...
	@echo Rust version:
	cargo fmt --version
	cargo fmt --all -- --check
	@echo Code is formatted!
	@echo ""

check-clippy:
	@echo Checking for clippy warnings...
	@echo Clippy version:
	cargo clippy --version
	cargo clippy --locked --all -- -D warnings
	@echo ""

# Find unused dependencies:
check-unused-deps:
	@echo Checking for unused dependencies...
	cargo udeps --version
	RUSTC_BOOTSTRAP=1 cargo udeps --all-targets --backend depinfo --locked
	@echo No unused dependencies found!
	@echo ""

check-cargo-deny:
	@echo "Checking for insecure dependencies..."
	cargo deny --version
	cargo deny --locked check -A warnings 
	@echo "No insecure dependencies found!"
	@echo ""

pre_lint:
	@echo "Running all lints..."
	@echo ""

lint: pre_lint check-fmt check-clippy check-unused-deps check-cargo-deny
	@echo "All checks passed!"
	@echo ""

test:
	@echo "Running all tests..."
	cargo --version
	cargo test --all --all-features --locked
	@echo "All tests passed!"
	@echo ""

test-all-feature-combinations:
	@echo "Running all tests with all feature combinations..."
	cargo test-all-features --version
	cargo test-all-features --locked
	@echo "All tests passed!"
	@echo ""

ci: lint test-all-feature-combinations
	@echo "All CI checks passed!"
	@echo ""

test-nix:
	@echo "Testing nix package build..."
	nix build
	@echo "Build succeeded"
	@echo "Making sure binary works..."
	./result/bin/prom-write --help
	@echo "Binary works!"
	@echo "Nix tests passed!"
	@echo ""

build-release:
	@echo "Building release binary..."
	cargo build --release
	@echo "Release binary built!"
	@echo "Sanity check of binary..."
	./target/release/prom-write --help
	@echo "Binary works!"
	@echo ""

publish-minor:
	@echo "Publishing to crates.io..."
	cargo release --version
	export CARGO_REGISTRIES_CRATES_IO_PROTOCOL=git
	cargo release --execute minor
	@echo "Published to crates.io!"
	@echo ""


.phony:
	echo hello
