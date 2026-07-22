UNAME_S := $(shell uname -s)
PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
DESTDIR ?=
CARGO_TARGET_DIR ?= target
INSTALL ?= install
SUDO ?= sudo
APP_ID := io.exaterm.Exaterm

ifeq ($(UNAME_S),Darwin)
APP_PACKAGE := exaterm-macos
else
APP_PACKAGE := exaterm-gtk
endif

.PHONY: all build build-app build-gtk build-macos build-release-linux run run-app run-gtk run-macos daemon check test test-workspace core-test core-check daemon-check system-install system-uninstall install install-linux install-linux-files uninstall uninstall-linux package package-macos package-macos-debug web web-deps web-run web-test clean help

all: build

build:
	cargo build -p exaterm-types -p exaterm-core -p exaterm-ui -p $(APP_PACKAGE) -p exatermd -p exaterm

build-app:
	cargo build -p $(APP_PACKAGE)

build-gtk:
	cargo build -p exaterm-gtk

build-macos:
	cargo build -p exaterm-macos

build-release-linux:
	@if [ "$(UNAME_S)" != "Linux" ]; then printf '%s\n' 'Error: the system install is supported on Linux only' >&2; exit 1; fi
	cargo build --release --target-dir "$(CARGO_TARGET_DIR)" -p exaterm -p exaterm-gtk -p exatermd

run: run-app

run-app: build-app
	cargo run -p $(APP_PACKAGE)

run-gtk: build-gtk
	cargo run -p exaterm-gtk

run-macos: build-macos
	cargo run -p exaterm-macos

daemon:
	cargo run -p exatermd

check:
	cargo check -p exaterm-types -p exaterm-core -p exaterm-ui -p $(APP_PACKAGE) -p exatermd

test:
	cargo test -p exaterm-types -p exaterm-core -p exaterm-ui -p $(APP_PACKAGE) -p exatermd

test-workspace: test

core-test:
	cargo test -p exaterm-core

core-check:
	cargo check -p exaterm-core

daemon-check:
	cargo check -p exatermd

install: install-linux

install-linux: build-release-linux
	$(MAKE) install-linux-files

install-linux-files:
	@if [ "$(UNAME_S)" != "Linux" ]; then printf '%s\n' 'Error: the system install is supported on Linux only' >&2; exit 1; fi
	$(INSTALL) -d "$(DESTDIR)$(BINDIR)"
	$(INSTALL) -m 0755 "$(CARGO_TARGET_DIR)/release/exaterm" "$(DESTDIR)$(BINDIR)/exaterm"
	$(INSTALL) -m 0755 "$(CARGO_TARGET_DIR)/release/exaterm-gtk" "$(DESTDIR)$(BINDIR)/exaterm-gtk"
	$(INSTALL) -m 0755 "$(CARGO_TARGET_DIR)/release/exatermd" "$(DESTDIR)$(BINDIR)/exatermd"
	$(INSTALL) -d "$(DESTDIR)$(DATADIR)/applications"
	$(INSTALL) -m 0644 "packaging/linux/$(APP_ID).desktop" "$(DESTDIR)$(DATADIR)/applications/$(APP_ID).desktop"
	$(INSTALL) -d "$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps"
	$(INSTALL) -m 0644 "assets/icons/hicolor/scalable/apps/$(APP_ID).svg" "$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/$(APP_ID).svg"
	$(INSTALL) -d "$(DESTDIR)$(DATADIR)/icons/hicolor/128x128/apps"
	$(INSTALL) -m 0644 "assets/icons/hicolor/128x128/apps/$(APP_ID).png" "$(DESTDIR)$(DATADIR)/icons/hicolor/128x128/apps/$(APP_ID).png"
	@if [ -z "$(DESTDIR)" ] && command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database "$(DATADIR)/applications"; fi

system-install: build-release-linux
	$(SUDO) $(MAKE) install-linux-files PREFIX="$(PREFIX)" BINDIR="$(BINDIR)" DATADIR="$(DATADIR)" CARGO_TARGET_DIR="$(abspath $(CARGO_TARGET_DIR))"

system-uninstall:
	$(SUDO) $(MAKE) uninstall-linux PREFIX="$(PREFIX)" BINDIR="$(BINDIR)" DATADIR="$(DATADIR)"

uninstall: uninstall-linux

uninstall-linux:
	@if [ "$(UNAME_S)" != "Linux" ]; then printf '%s\n' 'Error: the system install is supported on Linux only' >&2; exit 1; fi
	rm -f "$(DESTDIR)$(BINDIR)/exaterm" "$(DESTDIR)$(BINDIR)/exaterm-gtk" "$(DESTDIR)$(BINDIR)/exatermd"
	rm -f "$(DESTDIR)$(DATADIR)/applications/$(APP_ID).desktop"
	rm -f "$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/$(APP_ID).svg"
	rm -f "$(DESTDIR)$(DATADIR)/icons/hicolor/128x128/apps/$(APP_ID).png"
	@if [ -z "$(DESTDIR)" ] && command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database "$(DATADIR)/applications"; fi

package-macos:
	./scripts/package-macos.sh

package-macos-debug:
	./scripts/package-macos.sh --debug

ifeq ($(UNAME_S),Darwin)
package: package-macos
else
package:
	@echo "Error: packaging is not yet supported on $(UNAME_S)" >&2
	@exit 1
endif

web: web-deps
	cargo build -p exaterm-web -p exatermd

web-deps:
	cd crates/exaterm-web/frontend && npm install

web-run: web
	cargo run -p exaterm-web

web-test: web
	cd crates/exaterm-web && npx playwright test --reporter=line

clean:
	cargo clean

help:
	@printf '%s\n' \
		'make              Build the default app and daemon for this platform' \
		'make build-app    Build the native frontend package for this platform' \
		'make run          Build and run the native frontend package for this platform' \
		'make build-gtk    Build the GTK frontend explicitly' \
		'make run-gtk      Build and run the GTK frontend explicitly' \
		'make build-macos  Build the macOS frontend explicitly' \
		'make build-release-linux Build release binaries for a Linux install' \
		'make run-macos    Build and run the macOS frontend explicitly' \
		'make daemon       Run the daemon directly' \
		'make check        Check the default app and daemon for this platform' \
		'make test         Run the default app, core, UI, and daemon tests' \
		'make core-test    Run core library tests' \
		'make daemon-check Check the daemon package' \
		'make system-install Build, then install Linux app via sudo (PREFIX=/usr/local)' \
		'make system-uninstall Remove the Linux system installation via sudo' \
		'make install      Build and install directly; supports DESTDIR packaging' \
		'make uninstall    Remove an installation without privilege handling' \
		'make package      Build platform package (macOS: .app bundle)' \
		'make web          Build the web UI server' \
		'make web-run      Build and run the web UI server' \
		'make web-test     Run Playwright e2e tests for the web UI' \
		'make clean        Remove build artifacts'
