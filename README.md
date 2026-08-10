# pfac

Bundle a directory tree into a zip, then read one entry back out of it as a
seekable stream — no temp directory, and never the whole file in memory.

```rust
use std::io::{Read, Seek, SeekFrom};

// Uncompressed, so entries stay streamable.
pfac::create_bundle("./assets".as_ref(), "assets.pfac".as_ref(), false, None)?;

let mut bundle = pfac::Bundle::open("assets.pfac")?;

// Straight out of the zip. Nothing is unpacked, nothing is buffered whole.
let mut song = bundle.stream("music/take.flac")?;
song.seek(SeekFrom::Start(8192))?;
let mut frame = [0_u8; 4096];
song.read_exact(&mut frame)?;

// Small enough to want whole. Works on compressed and encrypted entries too.
let cover = bundle.read("art/cover.png")?;
```

## Install

```toml
[dependencies]
pfac = "0.8.0"
```

## The three functions

| | |
| --- | --- |
| `create_bundle(input_dir, output_file, compress, password)` | A directory tree in, one zip out. Subfolders at any depth, empty directories kept. |
| `Bundle::stream(name)` | A `Read + Seek + Send` handle onto one entry, reading in place. |
| `Bundle::read(name)` | The same entry, whole, as a `Vec<u8>`. Handles compressed and encrypted entries, which streaming does not. |

Files are streamed in with `io::copy` and out through a byte window, so bundling
or reading a 4 GB video costs a buffer, not 4 GB.

## Compression and streaming are exclusive

`stream` refuses a deflated entry with `Error::Compressed`.

Not because streaming compressed data needs memory — it does not; the zip layer
inflates chunk by chunk either way — but because it cannot **seek**. A deflate
stream is only readable forward from its start, so seeking backwards means
re-inflating from the beginning, and a `Seek` impl that quietly did that would be
a performance trap rather than a feature.

Since the libraries this exists to feed both want to seek, the useful bundle is
an uncompressed one. Media is already compressed, so `compress: false` usually
costs almost nothing in size:

```text
200 KB of incompressible audio  →  201 KB bundle stored, 201 KB deflated
```

Use `compress: true` for text, logs, or anything else that actually shrinks, and
reach for `read` on those entries.

## Feeding rusqlite

`rusqlite`'s `deserialize_read_exact` takes a `Read` and a length, both of which
`EntryReader` has — so a bundled database goes from the zip into SQLite's own
buffer with no temp file and no second copy in your process:

```rust
let stream = bundle.stream("meta/project.db")?;
let size = stream.len() as usize;

let mut connection = rusqlite::Connection::open_in_memory()?;
connection.deserialize_read_exact(rusqlite::MAIN_DB, stream, size, true)?;
```

Needs rusqlite's `serialize` feature. The database is read-only in this form, and
writes to it do not go back into the bundle. `tests/sqlite.rs` runs exactly this.

## Feeding audio decoders

`EntryReader` is `Read + Seek + Send`, which is what symphonia's `MediaSource`
asks for; `EntryReader::len` answers `byte_len`. claxon and anything else taking
a `Read` work directly. Wrap it in a `BufReader` — decoders read in small chunks,
and the reader issues a syscall per read.

## Passwords

`Some(password)` encrypts entry contents with **AES-256**. zip's legacy ZipCrypto
is broken and is never written.

Two things worth knowing before relying on it:

- **Entry names are not encrypted.** Zip encrypts contents and nothing else, so
  the file list, sizes, and timestamps stay readable without the password. If the
  names are the sensitive part, this is not the tool.
- **Encrypted entries cannot be streamed.** AES entries verify their HMAC only at
  the end, so a stream would hand out bytes that have not been authenticated yet.
  `read` reads to the end and can check it.

## Safety

- Entry names are normalised on the way in and refused if no platform could share
  them — `..`, absolute paths, drive letters, control characters, trailing dots
  or spaces, and Windows-reserved names like `CON`.
- `extract` checks every name again before joining it onto a directory, so a
  bundle built elsewhere cannot write outside the target.
- `read` will not allocate past `Bundle::max_read` (1 GiB by default), checked
  against the index *before* reading and against the running total while reading
   — the index is written by whoever built the bundle and can claim any size.
- `create_bundle` writes beside its destination and renames into place, so an
  interrupted run leaves any previous bundle intact.
- Symlinks are not followed and not stored: a link is a reference to one machine,
  and a bundle is meant to travel.

## What this crate does not do

No workspace, no metadata database, no dirty tracking, no CLI. A bundle is a zip
and a handful of ways into it.

## License

Licensed under the [MIT License](LICENSE).
