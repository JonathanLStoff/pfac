//! Bundle a directory tree into a zip, then read one entry back out of it as a
//! seekable stream — no temporary directory, and never the whole file in memory.
//!
//! ```text
//! ./assets/                          assets.pfac
//! ├── music/take.flac    ──create──▶  ┌──────────────┐
//! ├── art/cover.png                   │ one zip      │
//! └── meta/project.db                 └──────────────┘
//!                                            │
//!                        stream("music/take.flac") ──▶ Read + Seek + Send
//! ```
//!
//! # The three functions
//!
//! - [`create_bundle`] — a directory tree in, one zip out, with compression and
//!   a password as arguments.
//! - [`Bundle::stream`] — a [`Read`](std::io::Read) +
//!   [`Seek`](std::io::Seek) + [`Send`] handle onto one entry, reading straight
//!   out of the bundle.
//! - [`Bundle::read`] — the same entry, whole, in a `Vec<u8>`. Works on
//!   compressed and encrypted entries, which streaming does not.
//!
//! ```no_run
//! use std::io::{Read, Seek, SeekFrom};
//!
//! // Uncompressed, so entries stay streamable.
//! pfac::create_bundle("./assets".as_ref(), "assets.pfac".as_ref(), false, None)?;
//!
//! let mut bundle = pfac::Bundle::open("assets.pfac")?;
//!
//! // Straight out of the zip: no temp file, no full read.
//! let mut song = bundle.stream("music/take.flac")?;
//! song.seek(SeekFrom::Start(8192))?;
//! let mut frame = [0_u8; 4096];
//! song.read_exact(&mut frame)?;
//!
//! // Small enough to want whole.
//! let cover = bundle.read("art/cover.png")?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Compression and streaming are exclusive
//!
//! [`Bundle::stream`] refuses a deflated entry with [`Error::Compressed`].
//!
//! Not because streaming compressed data needs memory — it does not; the zip
//! layer inflates chunk by chunk either way — but because it cannot **seek**. A
//! deflate stream is only readable forward from its start, so seeking backwards
//! means re-inflating from the beginning, and a `Seek` implementation that
//! quietly did that would be a performance trap rather than a feature.
//!
//! Since both libraries this exists to feed want to seek, the useful bundle is
//! an uncompressed one. Media is already compressed anyway, so `compress: false`
//! usually costs almost nothing in size.
//!
//! # Feeding rusqlite
//!
//! SQLite needs a whole database, so this is [`Bundle::read`]'s territory — but
//! `rusqlite`'s `deserialize_read_exact` takes a [`Read`](std::io::Read) and a
//! length, both of which [`EntryReader`] has. The bytes go from the bundle into
//! SQLite's own buffer without a temp file and without a second copy in your
//! process:
//!
//! ```ignore
//! let stream = bundle.stream("meta/project.db")?;
//! let size = stream.len() as usize;
//!
//! let mut connection = rusqlite::Connection::open_in_memory()?;
//! connection.deserialize_read_exact(rusqlite::MAIN_DB, stream, size, true)?;
//! ```
//!
//! Needs rusqlite's `serialize` feature. The database is read-only in this form,
//! and writes to it do not go back into the bundle.
//!
//! # Feeding audio decoders
//!
//! [`EntryReader`] is `Read + Seek + Send`, which is what symphonia's
//! `MediaSource` asks for; [`EntryReader::len`] answers `byte_len`. claxon and
//! anything else taking a `Read` work directly. Wrap it in a
//! [`BufReader`](std::io::BufReader) — decoders read in small chunks, and the
//! reader issues a syscall per read.
//!
//! # What this crate does not do
//!
//! No workspace, no metadata database, no dirty tracking. A bundle is a zip and
//! a handful of ways into it.

#![warn(missing_docs)]

mod entry;
mod error;
mod names;
mod read;
mod write;

pub use entry::EntryReader;
pub use error::{Error, Result};
pub use names::normalize as normalize_name;
pub use read::{Bundle, DEFAULT_MAX_READ, EntryInfo};
pub use write::{BundleOptions, create_bundle, create_bundle_with};

#[cfg(test)]
mod assertions {
    use super::EntryReader;

    /// The bounds the docs promise, checked by the compiler rather than by me
    /// remembering. `Send` matters because audio decoding usually happens off
    /// the thread that opened the bundle.
    const fn _assert_bounds<T: std::io::Read + std::io::Seek + Send>() {}

    const _: () = _assert_bounds::<EntryReader>();
}
