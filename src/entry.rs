//! A [`Read`] + [`Seek`] window onto one stored entry, backed by its own file
//! handle.
//!
//! # Why not `ZipFile`
//!
//! The obvious implementation returns the `zip` crate's own `ZipFile<'a>`, which
//! borrows the `ZipArchive` mutably for as long as the stream lives. That allows
//! exactly one open entry at a time, and nothing else may touch the bundle
//! meanwhile — so no audio stream can be open while a database is queried, which
//! is most of the point of putting them in one file.
//!
//! An uncompressed entry is a contiguous run of bytes in the bundle, so there is
//! nothing to borrow: take its offset and length once, open a separate handle to
//! the same file, and serve reads from that window. Three things fall out —
//! [`Seek`], any number of concurrent streams, and [`Send`].
//!
//! # What it is not
//!
//! It does not check the CRC. The stored checksum covers the whole entry, so
//! verifying it means reading the whole entry, which is what a caller reaching
//! for a stream is avoiding. [`Bundle::verify`](crate::Bundle::verify) is there
//! for callers who want the guarantee and will pay for it.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

/// A seekable stream over one uncompressed entry.
///
/// Reads are clamped to the entry: position 0 is the entry's first byte, and
/// reads stop at its last rather than running on into the next entry.
///
/// ```no_run
/// use std::io::Read;
///
/// let mut bundle = pfac::Bundle::open("assets.pfac")?;
/// let mut song = bundle.stream("music/take.flac")?;
///
/// let mut header = [0_u8; 4];
/// song.read_exact(&mut header)?;
/// assert_eq!(&header, b"fLaC");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct EntryReader {
    file: File,
    /// Absolute offset of the entry's first data byte in the bundle.
    start: u64,
    /// Length of the entry. Stored entries are not compressed, so this is both
    /// the compressed and the uncompressed size.
    len: u64,
    /// Read position, relative to `start`.
    pos: u64,
}

impl EntryReader {
    pub(crate) fn new(file: File, start: u64, len: u64) -> Self {
        EntryReader {
            file,
            start,
            len,
            pos: 0,
        }
    }

    /// The entry's length in bytes.
    ///
    /// Worth having for its own sake, and required by anything that needs to
    /// know the size up front — `rusqlite`'s `deserialize_read_exact` and
    /// symphonia's `MediaSource::byte_len` both ask.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the entry is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// How many bytes are left after the current position.
    pub fn remaining(&self) -> u64 {
        self.len.saturating_sub(self.pos)
    }
}

impl Read for EntryReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.remaining();
        if remaining == 0 {
            return Ok(0);
        }

        // Never read past the entry, however big the caller's buffer is: the
        // bytes after it belong to the next entry, or to the central directory.
        let wanted = buffer.len().min(remaining as usize);

        // Seek every time rather than trusting the handle's position. Nothing
        // else holds this handle, but `read` is also reached through `Seek`, and
        // one authoritative position is cheaper to reason about than two.
        self.file.seek(SeekFrom::Start(self.start + self.pos))?;
        let read = self.file.read(&mut buffer[..wanted])?;

        self.pos += read as u64;

        Ok(read)
    }
}

impl Seek for EntryReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::End(offset) => self.len as i64 + offset,
            SeekFrom::Current(offset) => self.pos as i64 + offset,
        };

        if target < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek to a negative position",
            ));
        }

        // Seeking past the end is legal and mirrors a real file: the position
        // moves, and the next read returns nothing.
        self.pos = target as u64;

        Ok(self.pos)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A reader over the middle of a file, so the offset arithmetic is exercised
    /// rather than accidentally starting at zero.
    fn reader(prefix: &[u8], body: &[u8], suffix: &[u8]) -> (tempfile::NamedTempFile, EntryReader) {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(prefix).unwrap();
        file.write_all(body).unwrap();
        file.write_all(suffix).unwrap();
        file.flush().unwrap();

        let handle = File::open(file.path()).unwrap();
        let reader = EntryReader::new(handle, prefix.len() as u64, body.len() as u64);

        (file, reader)
    }

    #[test]
    fn reads_only_its_own_window() {
        let (_file, mut reader) = reader(b"PREFIX", b"body", b"SUFFIX");

        let mut all = Vec::new();
        reader.read_to_end(&mut all).unwrap();

        assert_eq!(all, b"body");
    }

    #[test]
    fn a_buffer_larger_than_the_entry_does_not_overrun() {
        let (_file, mut reader) = reader(b"PREFIX", b"body", b"SUFFIX");

        let mut buffer = [0_u8; 64];
        let read = reader.read(&mut buffer).unwrap();

        assert_eq!(read, 4);
        assert_eq!(&buffer[..read], b"body");
        assert_eq!(reader.read(&mut buffer).unwrap(), 0, "should be at the end");
    }

    #[test]
    fn seeking_lands_where_it_says() {
        let (_file, mut reader) = reader(b"PREFIX", b"0123456789", b"SUFFIX");

        assert_eq!(reader.seek(SeekFrom::Start(4)).unwrap(), 4);
        let mut two = [0_u8; 2];
        reader.read_exact(&mut two).unwrap();
        assert_eq!(&two, b"45");

        assert_eq!(reader.seek(SeekFrom::End(-2)).unwrap(), 8);
        reader.read_exact(&mut two).unwrap();
        assert_eq!(&two, b"89");

        reader.seek(SeekFrom::Start(0)).unwrap();
        assert_eq!(reader.seek(SeekFrom::Current(3)).unwrap(), 3);
        reader.read_exact(&mut two).unwrap();
        assert_eq!(&two, b"34");
    }

    #[test]
    fn seeking_backwards_is_free_and_repeatable() {
        let (_file, mut reader) = reader(b"", b"0123456789", b"");

        let mut first = [0_u8; 4];
        reader.read_exact(&mut first).unwrap();

        reader.rewind().unwrap();
        let mut again = [0_u8; 4];
        reader.read_exact(&mut again).unwrap();

        assert_eq!(first, again);
    }

    #[test]
    fn seeking_past_the_end_reads_nothing() {
        let (_file, mut reader) = reader(b"PREFIX", b"body", b"SUFFIX");

        reader.seek(SeekFrom::Start(1000)).unwrap();

        let mut buffer = [0_u8; 8];
        assert_eq!(reader.read(&mut buffer).unwrap(), 0);
    }

    #[test]
    fn seeking_before_the_start_is_an_error_not_a_wrap() {
        let (_file, mut reader) = reader(b"PREFIX", b"body", b"SUFFIX");

        assert!(reader.seek(SeekFrom::Current(-1)).is_err());
        // And the prefix stays unreachable.
        reader.rewind().unwrap();
        let mut all = Vec::new();
        reader.read_to_end(&mut all).unwrap();
        assert_eq!(all, b"body");
    }

    #[test]
    fn remaining_tracks_the_position() {
        let (_file, mut reader) = reader(b"", b"0123456789", b"");

        assert_eq!(reader.remaining(), 10);
        reader.seek(SeekFrom::Start(4)).unwrap();
        assert_eq!(reader.remaining(), 6);
        reader.seek(SeekFrom::Start(99)).unwrap();
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn an_empty_entry_is_readable_and_empty() {
        let (_file, mut reader) = reader(b"PREFIX", b"", b"SUFFIX");

        assert!(reader.is_empty());
        let mut all = Vec::new();
        reader.read_to_end(&mut all).unwrap();
        assert!(all.is_empty());
    }
}
