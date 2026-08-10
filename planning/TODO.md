# pfac — Roadmap

Bundle a directory tree into a zip, then read one entry back out of it as a
seekable stream — no temp directory, and never the whole file in memory.

```text
./assets/                          assets.pfac
├── music/take.flac    ──create──▶  ┌──────────────┐
├── art/cover.png                   │ one zip      │
└── meta/project.db                 └──────────────┘
                                           │
                       stream("music/take.flac") ──▶ Read + Seek + Send
```

**Scope.** Three functions and the safety around them. Everything the crate used
to be — workspace directories, a SQLite metadata store, locking, collation, a
CLI — was removed on purpose; see `CHANGELOG.md`. Do not add it back without a
reason that is about *this* crate.

---

## Where things stand

**Done:** see `CHANGELOG.md`. `create_bundle`, `Bundle::stream`, and
`Bundle::read` all work, with 45 tests over them. The rusqlite claim in the docs
is executed in `tests/sqlite.rs` rather than asserted.

**In Progress:**

- Nothing. Section 1's per-entry compression override is the next thing that
  changes what a caller can rely on.

---

## 0. Foundation

- [x] Library, no binary
- [x] `Cargo.toml`: `description`, `keywords`, `categories`, and a dependency
      list that is only what the three functions need
- [x] `README.md` describing what exists
- [x] Error type: `NoSuchEntry`, `Compressed`, `EncryptedStream`, `IsDirectory`,
      `Password`, `BadName`, `TooLarge`, `Io` (carrying the path), `Zip`
- [ ] CI that builds and tests. Only the publish workflow exists
- [ ] `cargo doc` warnings as errors in CI, since the docs make claims the tests
      check and the two should not drift

## 1. Writing — `create_bundle`

- [x] `create_bundle(input_dir, output_file, compress, password)`, plus
      `create_bundle_with` taking `BundleOptions` for the same three
- [x] `compress` selects Deflated or Stored; `level` is configurable
- [x] `password` uses AES-256. Never ZipCrypto
- [x] Subfolders at any depth, `/`-separated on every platform
- [x] Empty directories kept — `walkdir` plus `is_file()` alone drops them
- [x] Entries streamed in with `io::copy` behind a `BufReader`
- [x] Atomic: built beside the destination, flushed, fsynced, renamed
- [x] uid/gid normalised, symlinks skipped
- [ ] **Per-entry compression override.** Today `compress` is all-or-nothing, so
      turning it on makes *every* entry unstreamable — including a database or a
      FLAC that wanted to stay Stored. Wanted: a predicate or an extension table,
      so media stays stored while text is deflated
- [ ] Progress callback and cancellation. Bundling tens of gigabytes should be
      interruptible and should be able to say how far along it is
- [ ] Add to an existing bundle without rewriting it
- [ ] Zip64 is off (`large_file(false)`), so a single entry over 4 GiB will fail.
      Decide: always on, or set per entry from the file's size. Per entry is
      better — Zip64 headers cost bytes and break older readers
- [ ] mtimes are not preserved. Zip's DOS timestamps are two-second resolution
      with no timezone; decide whether that is worth carrying at all, and use the
      extended-timestamp field if so

## 2. Streaming — `Bundle::stream`

- [x] Returns `EntryReader`: `Read + Seek + Send`, its own file handle, does not
      borrow the bundle — so several can be open at once and one can stay open
      while other entries are read
- [x] Reads are clamped to the entry: a buffer larger than the entry does not
      run on into the next one
- [x] Refuses compressed entries (`Error::Compressed`) with a message naming the
      fix, and encrypted ones (`Error::EncryptedStream`)
- [x] Nothing allocates proportional to the entry
- [x] `len`, `is_empty`, `remaining`
- [x] Does not check the CRC, deliberately — that would cost a full read, which
      is what a caller reaching for a stream is avoiding. `Bundle::verify` is
      the opt-in
- [ ] **Streaming encrypted entries.** AES-CTR is seekable in principle, so a
      Stored+encrypted entry could be streamed. Blocked on two things: whether
      `zip` exposes enough to decrypt at an offset, and the fact that the HMAC
      is only verified at the end — so a stream hands out unauthenticated bytes.
      Do not do this without writing the tradeoff down
- [ ] `read_at`/positional reads, so one handle can serve concurrent readers
      without the seek-then-read pair being a race. Needs
      `FileExt::read_at`/`seek_read`, which are per-platform
- [ ] `Bundle::stream` re-opens the file each call. Fine at the current scale;
      revisit if something opens thousands

## 3. Reading whole — `Bundle::read`

- [x] Works on Stored, Deflated, and AES entries alike
- [x] Capped by `max_read` (1 GiB default), checked against the index *before*
      allocating and against the running total while reading — the index is
      attacker-controlled and can claim any size
- [x] `Bundle::verify` reads an entry and checks its CRC
- [ ] `read_into(&mut Vec<u8>)`, so a caller reading many entries can reuse one
      buffer

## 4. Listing and metadata

- [x] `names`, `names_under(prefix)`, `contains`, `len`, `is_empty`
- [x] `info(name)` and `entries()` returning `EntryInfo` — size, compressed size,
      stored, encrypted, is_dir — read from the index without decoding, so they
      work on encrypted entries without the password
- [x] `EntryInfo::streamable()`, so a caller can ask before trying
- [ ] Directory-shaped listing: immediate children of a prefix rather than
      everything beneath it
- [ ] `mtime` on `EntryInfo`, once section 1 decides whether mtimes are kept

## 5. Extraction

- [x] `extract(into)`, with every name checked before it is joined onto anything
- [ ] Extract one subtree rather than the whole bundle
- [ ] Free-space check before extracting
- [ ] Progress callback, as for writing

## 6. Safety

- [x] Names normalised on the way in, refused if no platform could share them
- [x] Names re-checked on the way out, including a `\` check that
      `Path::components` misses on Unix
- [x] `read` cannot be made to allocate past its ceiling
- [x] Symlinks not followed on write
- [ ] **The central directory and local headers can disagree.** A zip can name
      one file in the index and another in the header; readers trusting different
      halves see different archives. `data_start()` comes from the local header
      while the name comes from the index — confirm the `zip` crate reconciles
      these, and add a fixture that proves it
- [ ] Duplicate entry names in one bundle: decide, and make it first-wins or an
      error rather than whatever falls out
- [ ] Entries with a symlink attribute are stored by other tools even though this
      crate never writes them. `extract` should refuse them
- [ ] `has_overlapping_files` exists on `ZipArchive`; two entries sharing a byte
      range is a known trick. Worth checking on open, or offering as a method
- [ ] Passwords are not zeroized. The `String` a caller passes is theirs and we
      cannot reach it, so this would be half a guarantee — decide whether to take
      a zeroizing type instead of `&str`

## 7. Testing

45 tests: 14 unit, 23 integration in `tests/bundle.rs`, 3 in `tests/sqlite.rs`,
plus doctests.

- [x] Round trip across compressed/uncompressed × encrypted/plain, with
      subfolders and an empty directory
- [x] A streamed entry matches the original byte for byte
- [x] Seeking: forward, backward, from the end, past the end, before the start
- [x] A stream cannot read past its own entry
- [x] Two streams open at once, interleaved
- [x] A stream stays usable while the bundle is borrowed for another read
- [x] A stream moved to another thread
- [x] Streaming a compressed entry fails with advice; streaming an encrypted one
      fails
- [x] Wrong password, missing password, right password
- [x] `max_read` turns a huge entry into an error rather than an abort
- [x] Entry names are readable without the password — asserted so that if it ever
      stops being true, the docs get revisited
- [x] A real SQLite database, bundled and opened through the stream
- [ ] Hostile bundles built by hand: `../` names, absolute names, a symlink
      entry, a bomb, overlapping entries, a central directory that disagrees with
      a local header. The zip writer refuses to *produce* most of these, so the
      fixtures have to be assembled at the byte level
- [ ] Memory ceiling: stream an entry larger than any sane buffer and assert peak
      allocation stays flat. Behind an env guard so CI does not carry it
- [ ] A real FLAC fixture decoded through the stream with claxon or symphonia.
      The trait bounds are asserted at compile time today, which is not the same
      as a decoder actually seeking in one
- [ ] Property test over random directory trees for the round trip
- [ ] An entry over 4 GiB, once section 1 settles Zip64

## 8. Later

- [ ] A custom SQLite VFS over a Stored entry, for a read-write bundled database
      without pulling it into memory. `EntryReader` is most of the I/O half; the
      rest is locking and `xAccess`/`xOpen`
- [ ] `Read + Seek` over a bundle held in memory or behind a network fetch,
      rather than only a `File`
- [ ] Async reads
- [ ] Repair: salvage what is readable from a damaged bundle
