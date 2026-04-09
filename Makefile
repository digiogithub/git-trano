APP_NAME := git-trano
CRATE_DIR := .
TARGET_DIR := target
RELEASE_BIN := $(TARGET_DIR)/release/$(APP_NAME)

MUSL_TARGETS := x86_64-unknown-linux-musl aarch64-unknown-linux-musl
MUSL_BINS := $(foreach t,$(MUSL_TARGETS),$(TARGET_DIR)/$(t)/release/$(APP_NAME))

PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
DESTDIR ?=

CARGO ?= cargo
RUSTUP ?= rustup
STRIP ?= strip

.DEFAULT_GOAL := help

.PHONY: help check fmt clippy test build release static static-all static-x86_64 static-aarch64 install uninstall clean distclean print-vars

help:
	@echo "Available targets:"
	@echo "  make build           -> debug build"
	@echo "  make release         -> native release build"
	@echo "  make static          -> alias of static-x86_64"
	@echo "  make static-all      -> static build for x86_64 and aarch64 (musl)"
	@echo "  make static-x86_64   -> static build x86_64-unknown-linux-musl"
	@echo "  make static-aarch64  -> static build aarch64-unknown-linux-musl"
	@echo "  make check           -> cargo check"
	@echo "  make fmt             -> rustfmt (write mode)"
	@echo "  make clippy          -> clippy with warnings as errors"
	@echo "  make test            -> tests"
	@echo "  make install         -> install release binary to $(DESTDIR)$(BINDIR)"
	@echo "  make uninstall       -> remove binary from $(DESTDIR)$(BINDIR)"
	@echo "  make clean           -> cargo clean"
	@echo "  make distclean       -> clean + temporary files"
	@echo ""
	@echo "Useful variables:"
	@echo "  PREFIX=/usr          -> change install prefix"
	@echo "  DESTDIR=/tmp/pkgroot -> root for packaging"

check:
	$(CARGO) check --manifest-path $(CRATE_DIR)/Cargo.toml

fmt:
	$(CARGO) fmt --manifest-path $(CRATE_DIR)/Cargo.toml

clippy:
	$(CARGO) clippy --manifest-path $(CRATE_DIR)/Cargo.toml --all-targets -- -D warnings

test:
	$(CARGO) test --manifest-path $(CRATE_DIR)/Cargo.toml --all-targets

build:
	$(CARGO) build --manifest-path $(CRATE_DIR)/Cargo.toml

release:
	$(CARGO) build --manifest-path $(CRATE_DIR)/Cargo.toml --release
	@if [ -x "$(RELEASE_BIN)" ]; then \
		$(STRIP) "$(RELEASE_BIN)" || true; \
	fi
	@echo "Release binary: $(RELEASE_BIN)"

static: static-x86_64

static-x86_64:
	$(RUSTUP) target add x86_64-unknown-linux-musl
	$(CARGO) build --manifest-path $(CRATE_DIR)/Cargo.toml --release --target x86_64-unknown-linux-musl
	@if [ -x "$(TARGET_DIR)/x86_64-unknown-linux-musl/release/$(APP_NAME)" ]; then \
		$(STRIP) "$(TARGET_DIR)/x86_64-unknown-linux-musl/release/$(APP_NAME)" || true; \
	fi
	@echo "Static binary: $(TARGET_DIR)/x86_64-unknown-linux-musl/release/$(APP_NAME)"

static-aarch64:
	$(RUSTUP) target add aarch64-unknown-linux-musl
	$(CARGO) build --manifest-path $(CRATE_DIR)/Cargo.toml --release --target aarch64-unknown-linux-musl
	@if [ -x "$(TARGET_DIR)/aarch64-unknown-linux-musl/release/$(APP_NAME)" ]; then \
		$(STRIP) "$(TARGET_DIR)/aarch64-unknown-linux-musl/release/$(APP_NAME)" || true; \
	fi
	@echo "Static binary: $(TARGET_DIR)/aarch64-unknown-linux-musl/release/$(APP_NAME)"

static-all: static-x86_64 static-aarch64
	@echo "Generated static binaries:"
	@for b in $(MUSL_BINS); do \
		if [ -f "$$b" ]; then echo "  - $$b"; fi; \
	done

install: release
	install -d "$(DESTDIR)$(BINDIR)"
	install -m 0755 "$(RELEASE_BIN)" "$(DESTDIR)$(BINDIR)/$(APP_NAME)"
	@echo "Installed to: $(DESTDIR)$(BINDIR)/$(APP_NAME)"
	@echo "Use as git plugin: git trano ..."

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/$(APP_NAME)"
	@echo "Removed: $(DESTDIR)$(BINDIR)/$(APP_NAME)"

clean:
	$(CARGO) clean --manifest-path $(CRATE_DIR)/Cargo.toml

distclean: clean
	rm -rf .tmp dist

print-vars:
	@echo "APP_NAME=$(APP_NAME)"
	@echo "CRATE_DIR=$(CRATE_DIR)"
	@echo "TARGET_DIR=$(TARGET_DIR)"
	@echo "PREFIX=$(PREFIX)"
	@echo "BINDIR=$(BINDIR)"
	@echo "DESTDIR=$(DESTDIR)"
	@echo "MUSL_TARGETS=$(MUSL_TARGETS)"
