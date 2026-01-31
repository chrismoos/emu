BUILD_DIR := build
RELEASE_DIR := $(BUILD_DIR)/release
MACOS_APP := $(RELEASE_DIR)/$(APP_NAME).app
MACOS_CONTENTS := $(MACOS_APP)/Contents
MACOS_MACOS := $(MACOS_CONTENTS)/MacOS
MACOS_RESOURCES := $(MACOS_CONTENTS)/Resources

TARGET_MACOS_X86 := x86_64-apple-darwin
TARGET_MACOS_ARM := aarch64-apple-darwin
TARGET_WINDOWS := x86_64-pc-windows-gnu
TARGET_LINUX_X86 := x86_64-unknown-linux-musl
TARGET_LINUX_ARM := aarch64-unknown-linux-musl

ARCHIVE_BASE_NAME := emu

CARGO_FLAGS := --release

.PHONY: all clean macos macos-x86_64 macos-aarch64 windows linux linux-x86_64 linux-aarch64 wasm

macos: macos-x86_64 macos-aarch64
linux: linux-x86_64 linux-aarch64

all: macos linux windows

macos-x86_64:
	cargo build $(CARGO_FLAGS) --target $(TARGET_MACOS_X86)
	tar -czvf $(ARCHIVE_BASE_NAME)-$(TARGET_MACOS_X86).tar.gz -C target/$(TARGET_MACOS_X86)/release/ emu

macos-aarch64:
	cargo build $(CARGO_FLAGS) --target $(TARGET_MACOS_ARM)
	tar -czvf $(ARCHIVE_BASE_NAME)-$(TARGET_MACOS_ARM).tar.gz -C target/$(TARGET_MACOS_ARM)/release/ emu

MACOS_BINARY_PATH ?= target/$(TARGET_MACOS_NATIVE)/release/$(BINARY_NAME)

windows: 
	cargo build $(CARGO_FLAGS) --target $(TARGET_WINDOWS)
	tar -czvf $(ARCHIVE_BASE_NAME)-$(TARGET_WINDOWS).tar.gz -C target/$(TARGET_WINDOWS)/release/ emu.exe

linux-x86_64:
	cargo build $(CARGO_FLAGS) --target $(TARGET_LINUX_X86)
	tar -czvf $(ARCHIVE_BASE_NAME)-$(TARGET_LINUX_X86).tar.gz -C target/$(TARGET_LINUX_X86)/release/ emu

linux-aarch64:
	cargo build $(CARGO_FLAGS) --target $(TARGET_LINUX_ARM)
	tar -czvf $(ARCHIVE_BASE_NAME)-$(TARGET_LINUX_ARM).tar.gz -C target/$(TARGET_LINUX_ARM)/release/ emu

clean:
	cargo clean
	rm -rf $(BUILD_DIR)
	rm -rf dist
