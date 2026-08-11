# Convenience targets for lrfl.

BIN := lrfl

.PHONY: build test lint fmt fmt-check check dev install

build: SIGN_TARGET = target/release/$(BIN)
build:
	cargo build --release
	@$(SIGN)

test:
	cargo test

lint:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

check: fmt-check lint test

# Debug build re-signed with the stable pk-cli-codesign identity so macOS
# keychain "Always Allow" grants survive rebuilds (see cli-common/scripts).
dev: SIGN_TARGET = target/debug/$(BIN)
dev:
	cargo build
	@$(SIGN)

# `cargo install` ad-hoc signs, giving the binary a new code identity every
# time. macOS scopes keychain "Always Allow" grants to that identity, so an
# unsigned reinstall silently revokes them. Re-signing with the stable shared
# identity keeps one grant valid across every future install.
install: SIGN_TARGET = $${CARGO_INSTALL_ROOT:-$$HOME/.cargo}/bin/$(BIN)
install:
	cargo install --path . --force
	@$(SIGN)

# Shared re-signing step. No-ops with a note when the helper or identity is
# absent (CI, Linux, a machine that hasn't run setup-dev-signing.sh).
define SIGN
if [ -x "$$HOME/Dev/cli-common/scripts/dev-sign.sh" ]; then \
	"$$HOME/Dev/cli-common/scripts/dev-sign.sh" "$(SIGN_TARGET)"; \
else echo "cli-common/scripts/dev-sign.sh not found — $(SIGN_TARGET) left ad-hoc signed"; fi
endef
