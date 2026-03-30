.PHONY: all build build-gtk build-macos run run-gtk run-macos daemon check test test-workspace core-test core-check daemon-check clean help

all: build

build: build-gtk

build-gtk:
	cargo build -p exaterm-gtk -p exatermd

run: run-gtk

run-gtk: build-gtk
	cargo run -p exaterm-gtk

build-macos:
	cargo build -p exaterm-macos -p exatermd

run-macos: build-macos
	cargo run -p exaterm-macos

daemon:
	cargo run -p exatermd

check:
	cargo check -p exaterm-gtk

test:
	cargo test -p exaterm-gtk -p exaterm-ui

test-workspace:
	cargo test --workspace

core-test:
	cargo test -p exaterm-core

core-check:
	cargo check -p exaterm-core

daemon-check:
	cargo check -p exatermd

clean:
	cargo clean

help:
	@printf '%s\n' \
		'make            Build exaterm-gtk and exatermd' \
		'make run        Build and run the GTK app' \
		'make build-macos  Build exaterm-macos and exatermd' \
		'make run-macos  Build and run the macOS app' \
		'make daemon     Run the daemon directly' \
		'make check      cargo check for the GTK app' \
		'make test       Run app and UI tests' \
		'make test-workspace  Run the full workspace test suite' \
		'make core-test  Run core library tests' \
		'make daemon-check    Check the daemon package' \
		'make clean      Remove build artifacts'
