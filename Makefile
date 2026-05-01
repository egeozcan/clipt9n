SHELL := /usr/bin/env bash

APP_NAME := clipt9n
PREFIX ?= /usr/local
INSTALL_APP_DIR ?= /Applications
UNAME_S := $(shell uname -s)

.PHONY: help build compile test lint package install install-macos-reminder reset-macos-accessibility uninstall clean

help:
	@printf '%s\n' \
		'Targets:' \
		'  make build      Compile debug binary with cargo build' \
		'  make compile    Alias for build' \
		'  make test       Run cargo test' \
		'  make lint       Run cargo fmt check, cargo check, and platform lint' \
		'  make package    Build the platform package/app bundle' \
		'  make install    Install app bundle on macOS or binary/assets on Linux' \
		'  make install-macos-reminder  Open macOS Accessibility settings' \
		'  make reset-macos-accessibility  Reset clipt9n Accessibility permission and reopen Settings' \
		'  make uninstall  Remove files installed by make install' \
		'  make clean      Remove Cargo build artifacts'

build:
	cargo build

compile: build

test:
	cargo test

lint:
	cargo fmt --check
	cargo check
	scripts/lint-platform-discipline.sh

package:
ifeq ($(UNAME_S),Darwin)
	scripts/package-macos.sh
else ifeq ($(UNAME_S),Linux)
	scripts/package-linux.sh
else
	$(error Unsupported OS for package: $(UNAME_S))
endif

install: package
ifeq ($(UNAME_S),Darwin)
	-pkill -x "$(APP_NAME)"
	rm -rf "$(INSTALL_APP_DIR)/$(APP_NAME).app"
	cp -R "target/release/bundle/osx/$(APP_NAME).app" "$(INSTALL_APP_DIR)/"
	open "$(INSTALL_APP_DIR)/$(APP_NAME).app"
	@printf '\n%s\n%s\n%s\n\n' \
		'Installed and launched $(APP_NAME).' \
		'If hotkeys do not respond, macOS may require Accessibility permission again after a rebuilt local app install.' \
		'Run: make install-macos-reminder'
else ifeq ($(UNAME_S),Linux)
	install -d "$(DESTDIR)$(PREFIX)/bin"
	install -m 0755 "target/release/$(APP_NAME)" "$(DESTDIR)$(PREFIX)/bin/$(APP_NAME)"
	install -d "$(DESTDIR)$(PREFIX)/share/applications"
	install -m 0644 "target/release/package-linux/$(APP_NAME)/share/applications/dev.egecan.$(APP_NAME).desktop" "$(DESTDIR)$(PREFIX)/share/applications/dev.egecan.$(APP_NAME).desktop"
	install -d "$(DESTDIR)$(PREFIX)/share/icons/hicolor/256x256/apps"
	install -m 0644 "assets/icon-256.png" "$(DESTDIR)$(PREFIX)/share/icons/hicolor/256x256/apps/$(APP_NAME).png"
else
	$(error Unsupported OS for install: $(UNAME_S))
endif

install-macos-reminder:
ifeq ($(UNAME_S),Darwin)
	open 'x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility'
	@printf '%s\n' 'Enable clipt9n in Privacy & Security > Accessibility, then quit and reopen clipt9n.'
else
	@printf '%s\n' 'Accessibility settings are only needed on macOS.'
endif

reset-macos-accessibility:
ifeq ($(UNAME_S),Darwin)
	-pkill -x "$(APP_NAME)"
	tccutil reset Accessibility dev.egecan.$(APP_NAME)
	open 'x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility'
	@printf '%s\n' 'Re-enable /Applications/$(APP_NAME).app in Accessibility, then open it again.'
else
	@printf '%s\n' 'Accessibility permissions are only needed on macOS.'
endif

uninstall:
ifeq ($(UNAME_S),Darwin)
	rm -rf "$(INSTALL_APP_DIR)/$(APP_NAME).app"
else ifeq ($(UNAME_S),Linux)
	rm -f "$(DESTDIR)$(PREFIX)/bin/$(APP_NAME)"
	rm -f "$(DESTDIR)$(PREFIX)/share/applications/dev.egecan.$(APP_NAME).desktop"
	rm -f "$(DESTDIR)$(PREFIX)/share/icons/hicolor/256x256/apps/$(APP_NAME).png"
else
	$(error Unsupported OS for uninstall: $(UNAME_S))
endif

clean:
	cargo clean
