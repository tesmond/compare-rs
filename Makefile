# Builds the macOS app bundle for compare-rs.
#
#   make app        Release build packaged as target/macos/CompareRS.app
#   make run        Build the bundle and open it
#   make install    Copy the bundle to /Applications (override with INSTALL_DIR=...)
#   make dmg        Disk image with the app and an /Applications link
#   make universal  Same as `app`, but one binary for Apple silicon and Intel
#   make test       Run all tests
#   make lint       rustfmt check and clippy with warnings as errors
#   make hooks      Use the repo's git hooks (gitleaks secret scan before each commit)
#   make clean      Remove cargo build output, including the bundle
#
# Variables can be overridden, for example:
#   make app APP_NAME="MyCompare" BUNDLE_ID=com.example.compare \
#       SIGN_IDENTITY="Developer ID Application: ..."

APP_NAME      ?= CompareRS
BUNDLE_ID     ?= dev.compare-rs.app
MIN_MACOS     ?= 11.0
# "-" signs ad hoc, which is enough to run locally. Use a Developer ID to distribute.
SIGN_IDENTITY ?= -
INSTALL_DIR   ?= /Applications

ifeq ($(strip $(APP_NAME)),)
$(error APP_NAME must not be empty)
endif

EXECUTABLE := compare-rs
VERSION    := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
CARGO      ?= cargo

OUT_DIR    := target/macos
APP_BUNDLE := $(OUT_DIR)/$(APP_NAME).app
CONTENTS   := $(APP_BUNDLE)/Contents
DMG        := $(OUT_DIR)/$(APP_NAME)-$(VERSION).dmg
DMG_STAGE  := $(OUT_DIR)/dmg-staging
ICONSET    := $(OUT_DIR)/AppIcon.iconset

PLIST_TEMPLATE := packaging/macos/Info.plist
# Optional: a 1024x1024 PNG here becomes the app icon.
ICON_PNG       := packaging/macos/AppIcon.png
ICON_SIZES     := 16 32 128 256 512

RELEASE_BIN   := target/release/$(EXECUTABLE)
UNIVERSAL_BIN := target/universal/$(EXECUTABLE)
ARM_TARGET    := aarch64-apple-darwin
X86_TARGET    := x86_64-apple-darwin

.PHONY: all app universal bundle run install dmg test lint hooks clean

all: app

# Cargo decides what needs rebuilding, so it always runs.
app:
	$(CARGO) build --release -p compare-app
	@$(MAKE) --no-print-directory bundle BIN=$(RELEASE_BIN)

universal:
	rustup target add $(ARM_TARGET) $(X86_TARGET)
	$(CARGO) build --release -p compare-app --target $(ARM_TARGET)
	$(CARGO) build --release -p compare-app --target $(X86_TARGET)
	@mkdir -p $(dir $(UNIVERSAL_BIN))
	lipo -create -output $(UNIVERSAL_BIN) \
		target/$(ARM_TARGET)/release/$(EXECUTABLE) \
		target/$(X86_TARGET)/release/$(EXECUTABLE)
	@$(MAKE) --no-print-directory bundle BIN=$(UNIVERSAL_BIN)

# Assembles and signs the bundle from $(BIN). Called by `app` and `universal`.
bundle:
	@test -n "$(BIN)" || { echo "bundle: BIN is not set; run 'make app'"; exit 1; }
	@echo "Bundling $(APP_BUNDLE) (version $(VERSION))"
	@rm -rf "$(APP_BUNDLE)"
	@mkdir -p "$(CONTENTS)/MacOS" "$(CONTENTS)/Resources"
	@cp "$(BIN)" "$(CONTENTS)/MacOS/$(EXECUTABLE)"
	@sed -e 's|@APP_NAME@|$(APP_NAME)|g' \
	     -e 's|@EXECUTABLE@|$(EXECUTABLE)|g' \
	     -e 's|@BUNDLE_ID@|$(BUNDLE_ID)|g' \
	     -e 's|@VERSION@|$(VERSION)|g' \
	     -e 's|@MIN_MACOS@|$(MIN_MACOS)|g' \
	     "$(PLIST_TEMPLATE)" > "$(CONTENTS)/Info.plist"
	@plutil -lint -s "$(CONTENTS)/Info.plist"
	@printf 'APPL????' > "$(CONTENTS)/PkgInfo"
	@if [ -f "$(ICON_PNG)" ]; then \
		rm -rf "$(ICONSET)" && mkdir -p "$(ICONSET)" && \
		for size in $(ICON_SIZES); do \
			double=$$((size * 2)); \
			sips -z $$size $$size "$(ICON_PNG)" \
				--out "$(ICONSET)/icon_$${size}x$${size}.png" >/dev/null && \
			sips -z $$double $$double "$(ICON_PNG)" \
				--out "$(ICONSET)/icon_$${size}x$${size}@2x.png" >/dev/null || exit 1; \
		done && \
		iconutil -c icns "$(ICONSET)" -o "$(CONTENTS)/Resources/AppIcon.icns" && \
		rm -rf "$(ICONSET)"; \
	fi
	@codesign --force --options runtime --timestamp=none \
		--sign "$(SIGN_IDENTITY)" "$(APP_BUNDLE)"
	@codesign --verify --strict "$(APP_BUNDLE)"
	@echo "Built $(APP_BUNDLE)"

run: app
	open "$(APP_BUNDLE)"

install: app
	rm -rf "$(INSTALL_DIR)/$(APP_NAME).app"
	cp -R "$(APP_BUNDLE)" "$(INSTALL_DIR)/"
	@echo "Installed $(INSTALL_DIR)/$(APP_NAME).app"

dmg: app
	@rm -rf "$(DMG_STAGE)" "$(DMG)"
	@mkdir -p "$(DMG_STAGE)"
	cp -R "$(APP_BUNDLE)" "$(DMG_STAGE)/"
	ln -s /Applications "$(DMG_STAGE)/Applications"
	hdiutil create -volname "$(APP_NAME)" -srcfolder "$(DMG_STAGE)" -ov -format UDZO "$(DMG)"
	@rm -rf "$(DMG_STAGE)"
	@echo "Built $(DMG)"

test:
	$(CARGO) test --workspace

lint:
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings

hooks:
	git config core.hooksPath .githooks
	@command -v gitleaks >/dev/null || echo "Note: install gitleaks with 'brew install gitleaks'"
	@echo "Git hooks enabled from .githooks/"

clean:
	$(CARGO) clean
