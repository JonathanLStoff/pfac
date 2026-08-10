//! Opening a bundle and getting at what is in it.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use zip::{CompressionMethod, ZipArchive};

use crate::entry::EntryReader;
use crate::error::{Error, Result};
use crate::names;

/// Default ceiling on [`Bundle::read`]: 1 GiB.
///
/// Not a guess at how big entries get — it is the point past which reading one
/// whole into memory should be an explicit decision. Change it with
/// [`Bundle::set_max_read`].
pub const DEFAULT_MAX_READ: u64 = 1024 * 1024 * 1024;

/// What a bundle knows about one entry without reading it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryInfo {
    /// The entry's name: `/`-separated, relative to the bundle's root.
    pub name: String,
    /// Size once decompressed.
    pub size: u64,
    /// Size as stored.
    pub compressed_size: u64,
    /// Whether it is stored uncompressed, and so streamable.
    pub stored: bool,
    /// Whether its contents are encrypted.
    pub encrypted: bool,
    /// Whether it is a directory rather than a file.
    pub is_dir: bool,
}

impl EntryInfo {
    /// Whether [`Bundle::stream`] will work on this entry.
    pub fn streamable(&self) -> bool {
        self.stored && !self.encrypted && !self.is_dir
    }
}

/// An open bundle.
///
/// Opening reads the central directory — the index at the end of the file — and
/// nothing else. No entry is touched until it is asked for, and no temporary
/// directory is created at any point.
pub struct Bundle {
    path: PathBuf,
    archive: ZipArchive<BufReader<File>>,
    password: Option<Vec<u8>>,
    max_read: u64,
}

impl Bundle {
    /// Opens a bundle.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_inner(path.as_ref(), None)
    }

    /// Opens a bundle whose entries are encrypted.
    pub fn open_with_password(path: impl AsRef<Path>, password: &str) -> Result<Self> {
        Self::open_inner(path.as_ref(), Some(password.as_bytes().to_vec()))
    }

    fn open_inner(path: &Path, password: Option<Vec<u8>>) -> Result<Self> {
        let file = File::open(path).map_err(|error| Error::io(path, error))?;
        let archive =
            ZipArchive::new(BufReader::new(file)).map_err(|error| Error::zip(path, "", error))?;

        Ok(Bundle {
            path: path.to_path_buf(),
            archive,
            password,
            max_read: DEFAULT_MAX_READ,
        })
    }

    /// The bundle's path on disk.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many entries there are, directories included.
    pub fn len(&self) -> usize {
        self.archive.len()
    }

    /// Whether the bundle holds nothing at all.
    pub fn is_empty(&self) -> bool {
        self.archive.is_empty()
    }

    /// Every entry name, in the order the bundle stores them.
    ///
    /// Subfolder entries read as `textures/wood/diffuse.png`; directories carry
    /// a trailing `/`.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.archive.file_names()
    }

    /// Every entry name directly under `prefix`, and everything below it.
    ///
    /// ```no_run
    /// let bundle = pfac::Bundle::open("assets.pfac")?;
    /// for name in bundle.names_under("textures/") {
    ///     println!("{name}");
    /// }
    /// # Ok::<(), pfac::Error>(())
    /// ```
    pub fn names_under<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a str> {
        self.archive
            .file_names()
            .filter(move |name| name.starts_with(prefix) && *name != prefix)
    }

    /// Whether an entry exists.
    pub fn contains(&self, name: &str) -> bool {
        self.archive.index_for_name(name).is_some()
    }

    /// The index of an entry, or [`Error::NoSuchEntry`].
    ///
    /// Looking the index up separately is what lets the metadata calls reach an
    /// entry without asking the zip layer to decode it.
    fn index_of(&self, name: &str) -> Result<usize> {
        self.archive
            .index_for_name(name)
            .ok_or_else(|| Error::NoSuchEntry {
                archive: self.path.clone(),
                name: name.to_string(),
            })
    }

    /// What the index says about one entry, without reading it.
    ///
    /// Works on encrypted entries without the password — the index is not
    /// encrypted, and being able to ask "could this be streamed?" before trying
    /// is the point.
    pub fn info(&mut self, name: &str) -> Result<EntryInfo> {
        let index = self.index_of(name)?;

        // `by_name` builds a decoder and so refuses an encrypted entry outright;
        // `by_index_raw` hands back the entry untouched, which is all that is
        // needed to answer questions about it.
        let entry = self
            .archive
            .by_index_raw(index)
            .map_err(|error| Error::zip(&self.path, name, error))?;

        Ok(EntryInfo {
            name: entry.name().to_string(),
            size: entry.size(),
            compressed_size: entry.compressed_size(),
            stored: entry.compression() == CompressionMethod::Stored,
            encrypted: entry.encrypted(),
            is_dir: entry.is_dir(),
        })
    }

    /// Everything the index knows, for every entry.
    pub fn entries(&mut self) -> Result<Vec<EntryInfo>> {
        let names: Vec<String> = self.archive.file_names().map(str::to_string).collect();

        names.iter().map(|name| self.info(name)).collect()
    }

    /// The ceiling [`read`](Self::read) will not allocate past.
    pub fn max_read(&self) -> u64 {
        self.max_read
    }

    /// Sets the ceiling [`read`](Self::read) will not allocate past.
    pub fn set_max_read(&mut self, limit: u64) {
        self.max_read = limit;
    }

    /// Opens an entry as a seekable stream, reading straight out of the bundle.
    ///
    /// No temporary file, no unpacking, and the entry never lands in memory —
    /// bytes are read from the bundle on demand, so a 4 GB video costs whatever
    /// buffer the caller reads into and nothing more.
    ///
    /// The returned [`EntryReader`] is [`Read`] + [`Seek`](std::io::Seek) +
    /// [`Send`], has its own file handle, and does not borrow the bundle — so
    /// several can be open at once, and one can stay open while other entries
    /// are read.
    ///
    /// ```no_run
    /// use std::io::{Read, Seek, SeekFrom};
    ///
    /// let mut bundle = pfac::Bundle::open("assets.pfac")?;
    ///
    /// // Two streams, both live.
    /// let mut song = bundle.stream("music/take.flac")?;
    /// let mut cover = bundle.stream("art/cover.png")?;
    ///
    /// song.seek(SeekFrom::Start(1024))?;
    /// let mut buffer = [0_u8; 256];
    /// song.read_exact(&mut buffer)?;
    /// # let _ = &mut cover;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// - [`Error::NoSuchEntry`] if there is no such entry
    /// - [`Error::IsDirectory`] if it names a directory
    /// - [`Error::Compressed`] if the entry is deflated. Seeking inside a
    ///   deflate stream would mean re-inflating from the start every time, so a
    ///   compressed entry has no honest seekable form — write the bundle with
    ///   `compress: false`, or use [`read`](Self::read)
    /// - [`Error::EncryptedStream`] if the entry is encrypted. AES entries
    ///   authenticate only at the end, so a stream would hand out bytes that
    ///   have not been verified
    pub fn stream(&mut self, name: &str) -> Result<EntryReader> {
        let index = self.index_of(name)?;

        let (start, len) = {
            // Raw, not `by_name`: opening by name builds a decoder, which fails
            // on an encrypted entry before we can report *why* it cannot be
            // streamed. Nothing here needs decoding anyway — the whole point is
            // to find where the bytes are and leave them alone.
            let entry = self
                .archive
                .by_index_raw(index)
                .map_err(|error| Error::zip(&self.path, name, error))?;

            if entry.is_dir() {
                return Err(Error::IsDirectory {
                    name: name.to_string(),
                });
            }

            if entry.encrypted() {
                return Err(Error::EncryptedStream {
                    name: name.to_string(),
                });
            }

            let method = entry.compression();
            if method != CompressionMethod::Stored {
                return Err(Error::Compressed {
                    name: name.to_string(),
                    method: format!("{method}"),
                });
            }

            // Populated once the local header has been parsed, which opening the
            // entry above just did. The central directory's offset points at the
            // header, not the data, so this is the only honest source for it.
            let start = entry.data_start().ok_or_else(|| Error::Zip {
                path: self.path.clone(),
                source: zip::result::ZipError::InvalidArchive(
                    "entry has no recorded data offset".into(),
                ),
            })?;

            (start, entry.size())
        };

        // Its own handle: this is what frees the stream from borrowing the
        // archive, and what lets two of them exist at once.
        let file = File::open(&self.path).map_err(|error| Error::io(&self.path, error))?;

        Ok(EntryReader::new(file, start, len))
    }

    /// Reads a whole entry into memory.
    ///
    /// Works for compressed and encrypted entries alike — the zip layer inflates
    /// and decrypts on the way — which is what makes this the counterpart to
    /// [`stream`](Self::stream) rather than a lesser version of it.
    ///
    /// Refuses to allocate more than [`max_read`](Self::max_read), checked
    /// against the index *before* reading and against the running total while
    /// reading. Both, because the index is written by whoever built the bundle
    /// and an entry can claim any size at all.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchEntry`], [`Error::IsDirectory`], [`Error::Password`] for a
    /// wrong or missing password, and [`Error::TooLarge`] if the entry is over
    /// the ceiling.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        let limit = self.max_read;
        let path = self.path.clone();

        let mut entry = match self.password.clone() {
            Some(password) => self
                .archive
                .by_name_decrypt(name, &password)
                .map_err(|error| Error::zip(&path, name, error))?,
            None => self
                .archive
                .by_name(name)
                .map_err(|error| Error::zip(&path, name, error))?,
        };

        if entry.is_dir() {
            return Err(Error::IsDirectory {
                name: name.to_string(),
            });
        }

        // The claim, checked before a single byte is allocated.
        let declared = entry.size();
        if declared > limit {
            return Err(Error::TooLarge {
                name: name.to_string(),
                size: declared,
                limit,
            });
        }

        // Reserve what it claims, but read through a cap: a bundle that
        // understates an entry's size would otherwise grow the buffer past the
        // ceiling that was just checked.
        let mut buffer = Vec::with_capacity(declared as usize);
        let read = (&mut entry)
            .take(limit + 1)
            .read_to_end(&mut buffer)
            .map_err(|error| Error::io(&path, error))?;

        if read as u64 > limit {
            return Err(Error::TooLarge {
                name: name.to_string(),
                size: read as u64,
                limit,
            });
        }

        Ok(buffer)
    }

    /// Reads an entry and checks it against its stored CRC.
    ///
    /// [`stream`](Self::stream) deliberately does not do this: the checksum
    /// covers the whole entry, so verifying costs a full read, which is what a
    /// caller reaching for a stream is avoiding. This is here for when the
    /// guarantee is worth the read.
    pub fn verify(&mut self, name: &str) -> Result<()> {
        // `read_to_end` through the zip layer checks the CRC as it goes and
        // fails at the end if it does not match, so reading is verifying.
        self.read(name).map(|_| ())
    }

    /// Unpacks the whole bundle into a directory.
    ///
    /// Included because a bundle sometimes does have to become files again.
    /// Entry names are checked before anything is written, so a bundle built
    /// elsewhere cannot place a file outside `into`.
    ///
    /// # Errors
    ///
    /// [`Error::BadName`] for an entry that climbs out of `into`, plus anything
    /// [`read`](Self::read) can fail with.
    pub fn extract(&mut self, into: &Path) -> Result<()> {
        std::fs::create_dir_all(into).map_err(|error| Error::io(into, error))?;

        let names: Vec<String> = self.archive.file_names().map(str::to_string).collect();

        for name in names {
            // Checked before it is joined onto anything: the name is a claim
            // made by whoever wrote the bundle.
            names::check(&name)?;

            let target = into.join(&name);
            if !target.starts_with(into) {
                return Err(Error::bad_name(&name, "resolves outside the target"));
            }

            if name.ends_with('/') {
                std::fs::create_dir_all(&target).map_err(|error| Error::io(&target, error))?;
                continue;
            }

            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
            }

            let bytes = self.read(&name)?;
            std::fs::write(&target, bytes).map_err(|error| Error::io(&target, error))?;
        }

        Ok(())
    }
}

impl Drop for Bundle {
    fn drop(&mut self) {
        // Best-effort, as in `BundleOptions`: overwrite rather than leave the
        // password sitting in freed memory.
        if let Some(password) = self.password.as_mut() {
            password.fill(0);
        }
    }
}

impl std::fmt::Debug for Bundle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Bundle")
            .field("path", &self.path)
            .field("entries", &self.archive.len())
            .field("encrypted", &self.password.is_some())
            .finish()
    }
}
