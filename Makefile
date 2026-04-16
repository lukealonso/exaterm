UNAME_S := $(shell uname -s)

ifeq ($(UNAME_S),Darwin)
APP_PACKAGE := exaterm-macos
else
APP_PACKAGE := exaterm-gtk
endif

.PHONY: all build build-app build-gtk build-macos run run-app run-gtk run-macos daemon check test test-workspace core-test core-check daemon-check package package-macos package-macos-debug clean help

all: build

build:
	cargo build -p exaterm-types -p exaterm-core -p exaterm-ui -p $(APP_PACKAGE) -p exatermd

build-app:
	cargo build -p $(APP_PACKAGE)

build-gtk:
	cargo build -p exaterm-gtk

build-macos:
	cargo build -p exaterm-macos

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
		'make run-macos    Build and run the macOS frontend explicitly' \
		'make daemon       Run the daemon directly' \
		'make check        Check the default app and daemon for this platform' \
		'make test         Run the default app, core, UI, and daemon tests' \
		'make core-test    Run core library tests' \
		'make daemon-check Check the daemon package' \
		'make package      Build platform package (macOS: .app bundle)' \
		'make clean        Remove build artifacts'
