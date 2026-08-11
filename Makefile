# Build, test, and release pfac.
#
#   make            list every target
#   make test       run the test suite
#   make release    prepare a release in the working tree
#
# pfac is a library with no binary. Add example targets here as they appear,
# one each, so a target can carry the fixture or feature its example needs.

CARGO ?= cargo
# Where `make example` writes. Any extension works; with none, the example
# applies the crate's default (.pfac) through `with_default_extension`.
#   make example OUT=target/data.zip
OUT ?=
# Deflate the example bundle. Entries then need `read` rather than `stream`.
COMPRESS ?=
# Encrypt the example bundle with AES-256.
PASSWORD ?=
# `make icon` prints what registering an icon would do and stops. Set APPLY=1 to
# carry it out — it writes to the registry (Windows) or the MIME database
# (Linux), per user, no administrator rights needed.
APPLY ?=
EXT ?=
# The icon to register. With none, the example takes whichever of
# tests/data/pfac.{ico,icns,png} this desktop reads.
ICON ?=
# What the association is filed under: a ProgID on Windows, the MIME package
# name on Linux. Only has to be unique.
APP_ID ?=
APP_BUNDLE ?=
# The version to cut: make release v=0.9.0
v ?=

# Debug by default — nothing here is latency-sensitive. Override it when
# timing the archive work, since comparing compressors on a debug build
# measures the wrong thing: make build PROFILE=--release
PROFILE ?=

.DEFAULT_GOAL := help
.PHONY: help build test check fmt clippy doc clean release example example-all icon \
        require-cargo require-manifest require-version require-data

# --- checks ---------------------------------------------------------------
# Each is a guard the real targets depend on. They fail with a sentence saying
# what to do, rather than letting cargo fail with something longer.

require-cargo:
	@command -v $(CARGO) >/dev/null 2>&1 || { \
		echo "error: $(CARGO) not found. Install Rust from https://rustup.rs"; \
		exit 1; }

# The manifest has to parse before anything else means anything.
require-manifest: require-cargo
	@$(CARGO) metadata --no-deps --format-version 1 >/dev/null 2>&1 || { \
		echo "error: Cargo.toml does not parse. Run '$(CARGO) metadata' to see why."; \
		exit 1; }

# The example bundles this directory; without it there is nothing to show.
require-data:
	@test -d tests/data || { \
		echo "error: tests/data is missing — the example bundles it."; \
		exit 1; }

# `v` is the whole input to `make release`, so it is checked before anything is
# rewritten — a half-applied version bump is worse than a refusal.
require-version:
	@test -n "$(v)" || { \
		echo "error: no version. Usage: make release v=0.9.0"; \
		exit 1; }
	@printf '%s' '$(v)' \
		| grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$$' || { \
		echo "error: '$(v)' is not a semver version (x.y.z)"; \
		exit 1; }
	@test -f CHANGELOG.md || { \
		echo "error: no CHANGELOG.md to cut a release section in."; \
		exit 1; }
	@grep -Fqx '## Release $(v)' CHANGELOG.md && { \
		echo "error: CHANGELOG.md already has a 'Release $(v)' section."; \
		exit 1; } || true
	@# There has to be something to release, and it has to be checked here
	@# rather than mid-recipe: by then the version bump has already landed and
	@# a failure would leave the tree half-released.
	@awk ' \
		state == "pending" && /^## Release / { state = "tail" } \
		state == "pending" && NF { found = 1 } \
		state == "" && /^\*\*Done\*\*[[:space:]]*$$/ { state = "pending" } \
		END { if (state == "") exit 2; if (!found) exit 3 } \
	' CHANGELOG.md; \
	case $$? in \
		0) ;; \
		2) echo "error: CHANGELOG.md has no '**Done**' line to move items out of."; exit 1 ;; \
		3) echo "error: nothing under '**Done**' in CHANGELOG.md — no release to cut."; exit 1 ;; \
		*) echo "error: could not read CHANGELOG.md"; exit 1 ;; \
	esac

# --- examples -------------------------------------------------------------

## example: bundle ./tests/data and read it back (make example OUT=target/x.zip)
example: require-manifest require-data
	$(CARGO) run $(PROFILE) --example bundle_dir -- $(OUT)

## example-all: the same data as stored, deflated, and encrypted, side by side
example-all: require-manifest require-data
	@echo "==> stored (streamable), default .pfac extension"
	@$(CARGO) run $(PROFILE) --quiet --example bundle_dir -- target/example/stored
	@echo ""
	@echo '==> deflated (.zip) — smaller, but `stream` refuses the entries'
	@COMPRESS=1 $(CARGO) run $(PROFILE) --quiet --example bundle_dir -- target/example/deflated.zip
	@echo ""
	@echo '==> AES-256 (.bundle) — any extension, entries need `read`'
	@PASSWORD=hunter2 $(CARGO) run $(PROFILE) --quiet --example bundle_dir -- target/example/secret.bundle

## icon: show what registering a file-type icon would do (APPLY=1 to do it)
icon: require-manifest require-data
	$(CARGO) run $(PROFILE) --example register_icon

# --- build and test -------------------------------------------------------

## build: compile the crate
build: require-manifest
	$(CARGO) build $(PROFILE)

## test: run the test suite
test: require-manifest
	$(CARGO) test

## check: compile-check the crate and every target
check: require-manifest
	$(CARGO) check
	$(CARGO) check --all-targets

## fmt: format the source
fmt: require-cargo
	$(CARGO) fmt

## clippy: lint, with warnings as errors
clippy: require-manifest
	$(CARGO) clippy --all-targets -- -D warnings

## doc: build the documentation
doc: require-manifest
	$(CARGO) doc --no-deps

## clean: remove build artefacts
clean:
	$(CARGO) clean

# --- release --------------------------------------------------------------
# Prepares a release in the working tree and stops there. Nothing here commits,
# tags, or pushes: `.github/workflows/publish.yml` publishes off a `rel-<v>`
# branch, so pushing is the one irreversible step and stays a deliberate act.

## release: set the version everywhere and cut a CHANGELOG section (make release v=0.9.0)
release: require-version require-manifest
	@# Only the [package] version. A blanket substitution would also rewrite
	@# the dependency versions further down the manifest.
	@awk -v ver='$(v)' ' \
		/^\[/ { in_pkg = ($$0 == "[package]") } \
		in_pkg && !done && /^version[[:space:]]*=/ { \
			print "version = \"" ver "\""; done = 1; next } \
		{ print } \
	' Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml
	@grep -Fqx 'version = "$(v)"' Cargo.toml || { \
		echo "error: Cargo.toml still does not say $(v) — is there a [package] version line?"; \
		exit 1; }
	@# The README's install snippets are the other place the version is
	@# written by hand, in both the bare and the with-features form.
	@sed -i.bak -E \
		-e 's/^pfac = "[^"]*"/pfac = "$(v)"/' \
		-e 's/^pfac = \{ version = "[^"]*"/pfac = { version = "$(v)"/' \
		README.md && rm -f README.md.bak
	@# Cargo.lock records the crate's own version too; resolving rewrites it.
	@$(CARGO) metadata --format-version 1 >/dev/null
	@# Everything under **Done** becomes the new release section, leaving
	@# **Done** empty for the next cycle. Exit codes rather than messages,
	@# because awk's stdout is the file being written.
	@awk -v ver='$(v)' ' \
		BEGIN { state = "head" } \
		state == "head" { \
			print; \
			if ($$0 ~ /^\*\*Done\*\*[[:space:]]*$$/) state = "pending"; \
			next } \
		state == "pending" && /^## Release / { state = "tail" } \
		state == "pending" { pending = pending $$0 "\n"; next } \
		{ tail = tail $$0 "\n" } \
		END { \
			if (state == "head") exit 2; \
			sub(/^\n+/, "", pending); sub(/\n+$$/, "", pending); \
			if (pending == "") exit 3; \
			print ""; print "## Release " ver; print ""; print pending; \
			if (tail != "") { sub(/\n+$$/, "", tail); print ""; print tail } \
		} \
	' CHANGELOG.md > CHANGELOG.md.tmp; \
	code=$$?; \
	if [ $$code -ne 0 ]; then rm -f CHANGELOG.md.tmp; fi; \
	case $$code in \
		0) mv CHANGELOG.md.tmp CHANGELOG.md ;; \
		2) echo "error: CHANGELOG.md has no '**Done**' line to move items out of."; exit 1 ;; \
		3) echo "error: nothing under '**Done**' in CHANGELOG.md — no release to cut."; exit 1 ;; \
		*) exit $$code ;; \
	esac
	@echo "==> $(v): Cargo.toml, Cargo.lock, README.md, CHANGELOG.md"
	@echo ""
	@echo "  Review the diff, then:"
	@echo "    git checkout -b rel-$(v) && git commit -am 'release $(v)' && git push -u origin rel-$(v)"
	@echo ""
	@echo "  The push publishes to crates.io and opens the GitHub release."

# --- help -----------------------------------------------------------------

help:
	@echo "pfac — Project Folder Automatic Collator"
	@echo ""
	@grep -E '^## ' $(MAKEFILE_LIST) \
		| sed -e 's/^## //' -e 's/:/:|/' \
		| awk -F'|' '{ printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2 }'
	@echo ""
	@echo "  A library: bundle a directory into a zip, then stream one entry"
	@echo "  back out of it. See planning/TODO.md for what is left."
	@echo ""
	@echo "  'make example' bundles ./tests/data. The extension is yours —"
	@echo "  .pfac is only what you get by not choosing:"
	@echo "    make example OUT=target/data.zip"
	@echo "    make example OUT=mybundle COMPRESS=1"
	@echo "    make example PASSWORD=hunter2"
	@echo ""
	@echo "  'make icon' prints what registering an icon for an extension would"
	@echo "  write, and changes nothing. 'make icon APPLY=1' carries it out."
	@echo "  It registers whichever of tests/data/pfac.{ico,icns,png} this"
	@echo "  desktop reads; ICON=, EXT=, APP_ID=, and APP_BUNDLE= override."
