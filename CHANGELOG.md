# Changelog

Finished work is moved out of `planning/TODO.md` and listed under **Done**
below as it lands (see `.claude/CLAUDE.MD`).

`make release v=x.y.z` empties **Done** into a new `## Release x.y.z` section
underneath it, so **Done** always holds exactly what is unreleased.

**Done**

- **`BundleWriter::update(bundle, &options)`: add to or replace entries in an
  existing bundle.** Entries written through it are added, or replace the entry
  of the same name; every other entry — directories, compressed, and encrypted
  ones included — is carried over with `raw_copy_file`, so nothing is
  decompressed, decrypted, or recompressed. It is still a rewrite of the file,
  built beside the original and renamed over it, so an interrupted update leaves
  the original as it was. Options apply to the entries written, so pass the
  bundle's own password. Its motivating caller is OrbitX saving a SQLite image
  back into `core.planet`. 4 tests in `tests/writer.rs`
- **`BundleWriter`: write an export straight into a bundle entry.**
  `BundleWriter::create(path, &options)` starts a bundle, `entry(name)` returns
  an `EntryWriter` that is `Write + Seek + Read`, and `EntryWriter::finish`
  lands it; `BundleWriter::finish` writes the index and renames into place,
  exactly as `create_bundle` does (which now runs on it). An exporter that takes
  a writer — atome's `to_flac_into`, vtome's `transcode_into` — writes into the
  bundle with no file made first. `Seek` and `Read` are there because exporters
  patch their headers and move their indexes, which a zip entry, written once
  and forward, cannot; so the bytes spool to an anonymous scratch file and are
  copied in on `finish`. Dropping an unfinished entry discards it, and a name
  already written is refused when the entry is opened rather than after the
  export. 9 tests in `tests/writer.rs`; `make example-export` runs
  `examples/export_into_bundle.rs`, which has no atome or vtome dependency so
  that CI's lone checkout still builds
- (ad hoc, outside pfac) Fixed compile errors in OrbitX's
  `src-tauri/src/project/mod.rs`: `&str` values concatenated with `+` (only
  `String + &str` is valid) in `get_asset_project_stream`, replaced with
  `format!`; a missing-paren typo on `project_name.clone)`; and use-after-move
  errors in `create_project` where `project`, `tmp_assets`, `seed_db`, and
  `out_path` were each consumed by `fs::copy` (or by binding `project` into
  `input`) and then reused afterward — fixed by passing references instead
- Icon registration is part of the crate's own surface, not just a submodule:
  `pfac::register_icon`, `pfac::unregister_icon`, `pfac::FileTypeIcon`, and
  `pfac::icon_in` are re-exported at the root, and the crate docs and the README
  carry the whole story rather than a pointer to it
- `icon_in(directory)` picks the icon a directory holds for *this* desktop, in
  the preference order of `required_icon_extensions()` — `.ico` on Windows,
  `.icns` on macOS, `.svg` before `.png` elsewhere. An application ships one
  icon per desktop side by side and stops writing a `cfg!` at every call site;
  `tests/data` is exactly such a directory (`pfac.ico`, `pfac.icns`, `pfac.png`)
  and is what `make icon` now registers from by default, whichever machine it
  runs on. A directory with nothing usable in it is an `Error::Io` naming the
  formats that would have worked
- **Breaking:** `app_id` is an argument to `FileTypeIcon::new` rather than a
  builder default. It was derived as `pfac.<extension>`, so two applications
  handling the same extension wrote the same registry key or MIME package: the
  second silently took over the first's icon, and `unregister` deleted an
  association it had not written. The builder method is gone and `app_id()` is
  now the getter. `FileTypeIcon::new("pfac", icon)` becomes
  `FileTypeIcon::new("pfac", icon, "com.myapp.bundle")`
- `make icon` takes `APP_ID=` alongside `EXT=`, `ICON=`, and `APP_BUNDLE=`, and
  depends on `require-data` since the fixtures are now what it defaults to
- `pfac::icon` registers a file-type icon with the desktop, switching mechanism
  by platform: `HKCU\Software\Classes` plus `SHChangeNotify` on Windows, a
  freedesktop MIME package and hicolor icon plus `update-mime-database` on
  Linux, and `CFBundleDocumentTypes` in an application bundle's `Info.plist`
  plus `lsregister` on macOS. Everything is per user; nothing needs elevation
- Split into `plan` and `apply` rather than one function that just does it.
  Registering edits the registry or the MIME database, so `plan` computes every
  key, file, and command and touches nothing, `apply` carries it out, and
  `register` is the two together. A `Plan` prints itemised, which also makes the
  logic testable without a machine to modify — and `make icon` shows the plan for
  the current host while `make icon APPLY=1` runs it
- The icon format is enforced per platform. **`.ico` is Windows-only**; macOS
  reads `.icns` and the freedesktop desktops read `.png` or `.svg`, so handing
  over the wrong one is `Error::IconFormat` rather than a registration that
  silently never draws. `required_icon_extensions()` answers at runtime
- macOS cannot register a bare extension at all: Launch Services takes document
  icons from an application bundle, and there is no per-extension icon API. So
  `FileTypeIcon::app_bundle` is required there and `plan` fails with
  `Error::Unsupported` naming the alternative, rather than reporting success for
  something that did nothing
- Compile-checked and clippy-clean for `x86_64-pc-windows-msvc`,
  `x86_64-unknown-linux-gnu`, and `aarch64-apple-darwin`. Cross-checking caught a
  real bug: `SHCNE_ASSOCCHANGED` is `u32` while `SHChangeNotify` takes `i32`.
  The PlistBuddy sequence was verified by running it against a scratch bundle and
  linting the result with `plutil`
- New error variants: `IconFormat`, `Unsupported`, `Shell`
- The Windows registry crates are declared under `[target."cfg(windows)"]`, so a
  macOS or Linux build resolves neither them nor their `windows-sys` tree
- `zip` is now `default-features = false`. Its defaults pull bzip2, zstd, lzma,
  ppmd, and xz — the first two building C libraries from source — and this crate
  only ever writes Stored or Deflated. The tree is pure Rust again, and
  cross-compiling stopped needing a C toolchain for a decoder nothing uses. The
  trade: a zip made elsewhere with one of those methods now fails to read with a
  clear "unsupported compression"
- `examples/bundle_dir.rs`, run by `make example`: bundles `./tests/data`, lists
  what went in, streams a FLAC out of the bundle with a seek to the middle and
  to the end, holds two streams open at once, and reads the smallest entry
  whole — without extracting anything. `make example-all` runs the same data
  three ways (stored, deflated, AES-256) so the differences are side by side
- **The extension is explicitly the caller's.** Nothing in the crate reads a
  file's name — a bundle is identified by zip's own central directory — so
  `.pfac`, `.zip`, `.bundle`, `.myapp-project` and no extension at all are
  equally valid, and renaming a bundle afterwards does not stop it opening.
  `DEFAULT_EXTENSION` is `pfac` as a convention and `with_default_extension`
  applies it to a path that has none, since `create_bundle` writes exactly the
  path it is given rather than silently saving somewhere else.
  `tests/extension.rs` round-trips eight spellings including `assets.tar.gz`,
  a rename, and a bundle in a dotted directory
- Corrected a claim in the README and the crate docs: "media is already
  compressed, so storing costs almost nothing" is only sometimes true. The
  fixtures in `tests/data` are synthetic tones, and deflate takes them from
  234 KB to 35 KB — so the docs now give both numbers and say to measure
- **Scope change: pfac is now a zip bundling library and nothing else.** Three
  functions — `create_bundle`, `Bundle::stream`, `Bundle::read` — plus the
  listing and safety around them. Everything below this entry describes the
  crate it used to be, and none of that code is left
- `create_bundle(input_dir, output_file, compress, password)` walks a directory
  tree into one zip. Subfolders at any depth, `/`-separated on every platform;
  empty directories kept, which `walkdir` plus `is_file()` alone would drop;
  entries streamed in with `io::copy` so a 4 GB asset costs a buffer rather than
  4 GB. `create_bundle_with` takes the same settings as a struct
- `Bundle::stream(name)` returns an `EntryReader`: `Read + Seek + Send` over the
  entry's byte range, reading straight out of the bundle with no temp directory
  and nothing buffered whole. It carries **its own file handle** rather than
  borrowing the archive, which is what lets several streams be open at once, one
  stay open while other entries are read, and one move to another thread. The
  `zip` crate's own `ZipFile<'a>` can do none of those — it borrows the archive
  mutably for the stream's whole life
- Streaming refuses compressed entries. The reason is **`Seek`, not memory**: the
  zip layer inflates chunk by chunk either way, but a deflate stream is only
  readable forward from its start, so seeking backwards would mean re-inflating
  from the beginning. A `Seek` impl that quietly did that would be a performance
  trap. The error names the fix rather than only reporting failure
- Streaming refuses encrypted entries too: AES entries verify their HMAC only at
  the end, so a stream would hand out bytes that have not been authenticated
- `Bundle::read(name)` takes an entry whole, and handles compressed and encrypted
  entries — the counterpart to streaming rather than a lesser version of it. It
  will not allocate past `max_read` (1 GiB by default), checked against the index
  *before* reading and against the running total while reading, because the index
  is written by whoever built the bundle and can claim any size at all
- `info`/`entries` read metadata through `by_index_raw`, so `EntryInfo::streamable()`
  can be asked about an encrypted entry without the password — `by_name` builds a
  decoder and refuses such an entry before it can be described
- Passwords use AES-256. ZipCrypto is never written; zip's legacy scheme is
  broken and writing it would offer a guarantee that does not hold. Documented,
  and asserted in a test: **entry names are not encrypted** by zip, so the file
  list, sizes, and timestamps stay readable without the password
- Entry names are normalised on the way in and re-checked on the way out, keeping
  the cross-platform rules from the previous design — `..`, absolutes, drive
  letters, control characters, trailing dots or spaces, Windows-reserved names —
  plus a backslash check, since `Path::components` does not treat `\` as a
  separator on Unix and `..\..\evil` would otherwise pass on the way to Windows
- `create_bundle` keeps the atomic write: built beside the destination, flushed
  through both buffers, renamed into place, so an interrupted run leaves any
  previous bundle intact
- Dependencies are only what the three functions need: `zip`, `walkdir`,
  `tempfile`, `thiserror`. Dropped `tar`, `zstd`, `blake3`, `directories`,
  `serde_json`, and `rusqlite` — the last of which returns as a dev-dependency
  only, to execute the docs' claim rather than assert it
- 53 tests. `tests/sqlite.rs` bundles a real SQLite database and opens it through
  `deserialize_read_exact`, which takes a `Read` and a length — both of which
  `EntryReader` has — so the bytes go from the bundle into SQLite's own buffer
  with no temp file and no second copy
- Removed: `Project`, `Workspace`, the SQLite metadata store, the collator, the
  advisory locking, the manifest and format versioning, and the CLI

## Release 0.8.0 (superseded design)

Everything below built a project-file format on tar + zstd, with a working
directory and a metadata database. None of this code remains; it is kept here
because it is where the reasoning above came from.

- The project lifecycle, end to end: `Project::create`, `open`, `open_dir`,
  `save`, `save_as`, `close`, `discard`. A project is a directory while it is
  open and one zstd-compressed tar when it is not, with SQLite metadata beside
  assets that stay ordinary files. One handle owns the workspace *and* the
  connection, so the directory cannot be dropped while a connection to a file
  inside it is still open
- `save` checkpoints the database before archiving. In WAL mode a committed
  transaction may live only in `project.db-wal`, so copying `project.db` at that
  moment captures a file missing its newest commits — and since the sidecars are
  not archived, those commits would be gone for good. `wal_checkpoint(TRUNCATE)`
  runs first, and `tests/durability.rs` saves with the connection still open —
  the state an application is in when the user hits save — and asserts the
  commit survives the round trip
- `save` is atomic. The archive is built beside its destination, flushed through
  both the `BufWriter` and the kernel, then renamed into place, so an
  interrupted save leaves the previous archive exactly as it was rather than a
  truncated file where the project used to be. Tested by failing a save against
  a read-only directory and comparing the previous archive byte for byte
- The workspace outlives the process (`Persistence::Durable`, under the user's
  data directory, keyed on a hash of the archive's absolute path so two projects
  named alike do not collide). A `TempDir` that deletes on drop makes every
  unsaved edit contingent on a clean exit; this makes a crash recoverable.
  `Persistence::Ephemeral` is the opt-in for reading a project without leaving a
  copy behind, and is what `pfac ls` and `pfac verify` use
- One project, one process: an **OS advisory lock** on `.pfac-lock`. The kernel
  releases it when the holder dies, so unlike a file holding a PID it cannot go
  stale and brick a project after one crash. The PID is still written in, but
  only so the error can name it. Needs no dependency — `File::try_lock` has been
  in std since 1.89
- Extraction assumes nothing about the archive, because a `.pfac` is a file that
  gets emailed around. Entries are checked component by component, links of any
  kind are refused outright — a symlink is the escape a path check alone does
  not close — and the extracted total and entry count are capped so a small
  archive cannot fill the disk. `tests/hostile.rs` builds ten such archives by
  hand, since pfac's own writer would never emit them
- Asset paths are normalised to one portable form and refused if no platform
  could share them: `..`, absolute, drive letters, control characters, trailing
  dots or spaces, and Windows-reserved names like `CON`. Checked on every
  platform, so a project written on Linux cannot fail to open on Windows
- The manifest is the archive's first entry, so `read_manifest` — and `pfac
  info` — costs one entry rather than a whole extraction. It carries the format
  version and a `min_reader`, and an archive from a newer pfac is refused with
  both numbers rather than half-unpacked. The database's `schema_version` works
  the same way
- Compression is zstd at level 3, configurable through `PackOptions`. A project
  is mostly already-compressed media, where a higher level spends real time for
  almost nothing — a 200 KB incompressible asset archives to 201 KB
- The collator: `import` (copying, hashing, registering, dedupe on content hash,
  and a numeric suffix rather than overwriting a name held by different bytes),
  `remove`, `scan` (orphans on disk, dangling rows, files that changed
  underneath us), and `collate` to apply all three
- A CLI over the library: `new`, `add`, `ls`, `info`, `verify`, `extract`,
  `pack`. Seven subcommands parsed by hand, which is less code than configuring
  a parser would be
- 54 tests — 17 unit, 35 integration across round-trip, durability, hostile
  archives, and collation, plus 2 doctests. Clean under
  `cargo clippy --all-targets -- -D warnings`
- The scaffolding is pfac's own rather than `atome`'s, from which it was copied.
  The `Makefile` had targets for playing audio files through a plugin chain, a
  `require-import` guard for a feature pfac does not have, and a `require-native`
  guard for building libfdk-aac and libopus; `make test` therefore failed before
  running anything. It now carries `build`, `run`, `test`, `check`, `fmt`,
  `clippy`, `doc`, `clean`, and `release`, and its help text describes pfac
- `publish.yml` no longer installs ALSA development headers — that step existed
  because cpal's Linux backend needs them, and pfac has no audio dependency —
  and tests without `--features import`, which does not exist here. The rest of
  the workflow reads the crate name from `cargo metadata`, so it needed nothing
- `make release` rewrites `pfac = "x.y.z"` in the README's install snippets
  rather than `atome = "x.y.z"`
- `planning/TODO.md`: the roadmap for the working-directory-plus-archive project
  format — SQLite metadata alongside assets on disk, compressed to a single file
  when closed
