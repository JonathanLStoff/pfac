# Changelog

Finished work is moved out of `planning/TODO.md` and listed under **Done**
below as it lands (see `.claude/CLAUDE.MD`).

`make release v=x.y.z` empties **Done** into a new `## Release x.y.z` section
underneath it, so **Done** always holds exactly what is unreleased.

**Done**

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
- 45 tests. `tests/sqlite.rs` bundles a real SQLite database and opens it through
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
