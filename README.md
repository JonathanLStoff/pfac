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

One more, for the application that owns the extension rather than the bundle:
`register_icon(&spec)` tells the desktop what to draw for `.pfac` — see
[Registering an icon for your extension](#registering-an-icon-for-your-extension).

## Compression and streaming are exclusive

`stream` refuses a deflated entry with `Error::Compressed`.

Not because streaming compressed data needs memory — it does not; the zip layer
inflates chunk by chunk either way — but because it cannot **seek**. A deflate
stream is only readable forward from its start, so seeking backwards means
re-inflating from the beginning, and a `Seek` impl that quietly did that would be
a performance trap rather than a feature.

Since the libraries this exists to feed both want to seek, the useful bundle is
usually an uncompressed one — and for already-compressed media that costs little:

```text
200 KB of incompressible audio  →  201 KB stored, 201 KB deflated
```

But measure rather than assume. `make example-all` bundles `tests/data`, whose
FLACs are synthetic tones and so still hugely redundant:

```text
233 KB of synthetic-tone FLAC   →  234 KB stored,  35 KB deflated
```

Real recordings sit much nearer the first case than the second. Compress when the
data actually shrinks and you can live with `read`; store when you want to seek.

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

## Writing an export straight into a bundle

`create_bundle` needs the files to exist first. For content that is *produced* —
a FLAC encoder, a video muxer — `BundleWriter` hands out a writer instead:

```rust
use pfac::{BundleOptions, BundleWriter};

let mut bundle = BundleWriter::create("project.pfac", &BundleOptions::stored())?;

let mut entry = bundle.entry("music/take.flac")?;
atome::export::to_flac_into("take.mp3", &mut entry)?;   // writes here, not to a path
entry.finish()?;                                         // only now is it in the bundle

bundle.finish()?;                                        // only now is the bundle on disk
```

`BundleWriter::update("project.pfac", &options)` does the same to an existing
bundle: the entries you write are added, or replace the ones of that name, and
the rest are carried over untouched (copied as stored — nothing is decrypted or
recompressed), then the bundle is renamed into place.

An `EntryWriter` is `Write + Seek + Read`, because exporters go back and patch
what they wrote — FLAC's header, WebM's sizes, MP4's index. A zip entry cannot:
it is written once, forward. So the bytes go to an anonymous scratch file and
are copied into the bundle by `finish`. That is a copy on disk, never a buffer
the size of the export. An entry that is dropped rather than finished is
discarded, so a failed export leaves nothing in the bundle; a name cannot be
written twice.

`make example-export` runs `examples/export_into_bundle.rs`, which writes two
exports into a bundle and streams one back out of it. `make example-update` runs
`examples/update_bundle.rs`, which adds a file to an asset folder of a bundle
that already exists and replaces another entry in it.

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

## Registering an icon for your extension

Part of the library, not a side script: `pfac::register_icon` (the `pfac::icon`
module in full) tells the desktop what to draw for a file extension, picking the
right mechanism per platform:

| Platform | Icon format | Where the association goes |
| --- | --- | --- |
| Windows | `.ico` | `HKCU\Software\Classes` — no admin rights |
| Linux | `.png` or `.svg` | freedesktop MIME database + hicolor icon theme |
| macOS | `.icns` | an application bundle's `Info.plist`, then `lsregister` |

Note the formats differ — **`.ico` is Windows-only**, and handing one to the
others is `Error::IconFormat` rather than a silent no-op.
`icon::required_icon_extensions()` answers at runtime, and `pfac::icon_in` goes
one better: point it at a directory holding one icon per desktop and it hands
back the one *this* machine can draw, so no caller needs a `cfg!` of its own.

Because this writes to the user's system, it comes apart into a plan and an
apply. `plan` computes every key, file, and command and touches nothing:

```rust
use pfac::{FileTypeIcon, icon_in};
use pfac::icon::{plan, apply};

// tests/data ships pfac.ico, pfac.icns, and pfac.png side by side.
let icon = icon_in("tests/data")?;

let spec = FileTypeIcon::new("pfac", icon, "com.myapp.bundle")
    .description("MyApp Bundle");

let steps = plan(&spec)?;
println!("{steps}");   // itemised; nothing has happened yet
apply(&steps)?;
```

`register_icon` is the two together, and `unregister_icon` undoes it. Everything
is per-user, so none of it needs elevation. `make icon` prints the plan for this
machine — using the `tests/data` icon for whichever desktop it is run on — and
`make icon APPLY=1` carries it out. `EXT=`, `ICON=`, `APP_ID=`, and
`APP_BUNDLE=` override the defaults.

**The app id is an argument, not a default.** It is what the association is
filed under — a ProgID on Windows, the MIME package file name on Linux — so it
only has to be unique, and reverse-DNS is the convention. Deriving it from the
extension would mean two applications that both handle `.pfac` writing the same
key: the second would silently take over the first's icon, and `unregister_icon`
would delete an association it never wrote.

**An icon is not an opener.** On its own the association only changes what the
file looks like. `FileTypeIcon::open_with(program)` also makes a double-click
start `program` with the file's path: Windows gets the ProgID's
`shell\open\command`, Linux a hidden `<app_id>.desktop` entry made the MIME
type's default with `xdg-mime`. `app_name` sets what the desktop calls the
program. macOS ignores both — the application bundle declares what it opens.

**macOS needs an application bundle.** Windows and Linux let a bare process
claim an extension; macOS takes document icons from an app bundle's
`Info.plist`, and there is no per-extension icon API to call. So
`FileTypeIcon::app_bundle` is required there, and without it `plan` fails with
`Error::Unsupported` explaining why rather than pretending.

## What this crate does not do

No workspace, no metadata database, no dirty tracking, no CLI. A bundle is a zip
and a handful of ways into it.

## License

Licensed under the [MIT License](LICENSE).
