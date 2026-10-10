//! Building a bundle from a directory tree.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;
use walkdir::WalkDir;
use zip::write::{FileOptions, SimpleFileOptions};
use zip::{AesMode, CompressionMethod, ZipWriter};

use crate::error::{Error, Result};
use crate::names;

/// How to write a bundle.
///
/// [`create_bundle`] takes the three settings positionally; this is the same
/// thing when there are more of them than reads well as arguments.
#[derive(Clone, Debug)]
pub struct BundleOptions {
    /// Deflate entries rather than storing them.
    ///
    /// **Compressed entries cannot be streamed** — see [`Bundle::stream`] — so
    /// this trades reading an entry in place for a smaller file. For a bundle of
    /// already-compressed media (FLAC, MP4, PNG) it costs time and saves close
    /// to nothing, which is why it is off by default.
    ///
    /// [`Bundle::stream`]: crate::Bundle::stream
    pub compress: bool,

    /// Encrypt entry contents with AES-256.
    ///
    /// **Entry names are not encrypted.** Zip encrypts contents and nothing
    /// else, so the file list, sizes, and timestamps stay readable without the
    /// password. If the names themselves are sensitive, this is not the tool.
    pub password: Option<String>,

    /// Deflate level, 0–9. Ignored when `compress` is false.
    pub level: i64,
}

impl Default for BundleOptions {
    fn default() -> Self {
        BundleOptions {
            compress: false,
            password: None,
            level: 6,
        }
    }
}

impl BundleOptions {
    /// Uncompressed and unencrypted: every entry streamable.
    pub fn stored() -> Self {
        Self::default()
    }

    /// Deflate everything. Entries will not be streamable.
    pub fn compressed() -> Self {
        BundleOptions {
            compress: true,
            ..Self::default()
        }
    }

    /// Encrypt with AES-256 under this password.
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// The zip-level options these settings describe.
    ///
    /// The lifetime is not decoration: `with_aes_encryption` *borrows* the
    /// password, so the options cannot outlive the `BundleOptions` they came
    /// from. `SimpleFileOptions` is `FileOptions<'static, ()>`, which is exactly
    /// what a borrowed password cannot satisfy.
    fn file_options(&self) -> FileOptions<'_, ()> {
        let method = if self.compress {
            CompressionMethod::Deflated
        } else {
            CompressionMethod::Stored
        };

        let options = SimpleFileOptions::default()
            .compression_method(method)
            // uid/gid and the writing machine's umask describe that machine, not
            // the bundle. Normalised so a round trip does not carry them.
            .unix_permissions(0o644)
            // Zip64 for anything over 4 GiB; `LargeFile` decides per entry.
            .large_file(false);

        let options = if self.compress {
            options.compression_level(Some(self.level))
        } else {
            options
        };

        match self.password.as_deref() {
            // AES-256 only. Never ZipCrypto: zip's legacy scheme is broken, and
            // writing it would be offering a guarantee that does not hold.
            Some(password) => options.with_aes_encryption(AesMode::Aes256, password),
            None => options,
        }
    }
}

/// The extension this crate suggests, without requiring it.
///
/// A convention and nothing more — see [`create_bundle`] on why no code here
/// looks at it.
pub const DEFAULT_EXTENSION: &str = "pfac";

/// Adds [`DEFAULT_EXTENSION`] to a path that has no extension at all.
///
/// A convenience for a caller who wants the convention applied — `create_bundle`
/// itself writes exactly the path it is given, since silently saving to a
/// different filename than the one asked for would be the worse surprise.
///
/// A path that already has an extension is returned untouched, whatever that
/// extension is.
///
/// ```
/// use std::path::Path;
/// use pfac::with_default_extension;
///
/// assert_eq!(with_default_extension("assets"), Path::new("assets.pfac"));
///
/// // Anything already extended is left alone.
/// assert_eq!(with_default_extension("assets.zip"), Path::new("assets.zip"));
/// assert_eq!(with_default_extension("v1.2/assets.bundle"), Path::new("v1.2/assets.bundle"));
/// ```
pub fn with_default_extension(path: impl Into<PathBuf>) -> PathBuf {
    let path = path.into();

    match path.extension() {
        Some(_) => path,
        None => path.with_extension(DEFAULT_EXTENSION),
    }
}

/// Bundles `input_dir` into a zip at `output_file`.
///
/// Subfolders come along at any depth, named `/`-separated relative to
/// `input_dir` — `textures/wood/diffuse.png`, whatever the platform. Empty
/// directories are kept.
///
/// Files are streamed in with [`io::copy`], so bundling a 4 GB video does not
/// need 4 GB of memory.
///
/// The bundle is built beside `output_file` and renamed into place, so an
/// interrupted run leaves any previous bundle exactly as it was rather than a
/// truncated file where it used to be.
///
/// # The extension is yours
///
/// `output_file` is written exactly as given. Nothing in this crate inspects a
/// file's name: [`Bundle::open`](crate::Bundle::open) identifies a bundle by
/// reading zip's own central directory, so `.pfac`, `.zip`, `.bundle`,
/// `.myapp-project`, or no extension at all are all equally valid, and renaming
/// one afterwards does not stop it opening.
///
/// [`DEFAULT_EXTENSION`] is `pfac` as a convention, and
/// [`with_default_extension`] applies it for callers who want it.
///
/// ```no_run
/// // Uncompressed, so every entry can be streamed back out.
/// pfac::create_bundle("./assets".as_ref(), "assets.pfac".as_ref(), false, None)?;
///
/// // Any other extension does the same thing.
/// pfac::create_bundle("./assets".as_ref(), "assets.zip".as_ref(), false, None)?;
/// pfac::create_bundle("./assets".as_ref(), "level-01.myapp".as_ref(), false, None)?;
///
/// // Compressed and encrypted. Entries will need `read`, not `stream`.
/// pfac::create_bundle("./docs".as_ref(), "docs.pfac".as_ref(), true, Some("hunter2"))?;
/// # Ok::<(), pfac::Error>(())
/// ```
///
/// # Errors
///
/// [`Error::BadName`] if a file's path cannot be represented portably,
/// [`Error::Io`] for filesystem trouble, [`Error::Zip`] if the archive cannot be
/// written.
pub fn create_bundle(
    input_dir: &Path,
    output_file: &Path,
    compress: bool,
    password: Option<&str>,
) -> Result<()> {
    let options = BundleOptions {
        compress,
        password: password.map(str::to_string),
        ..Default::default()
    };

    create_bundle_with(input_dir, output_file, &options)
}

/// [`create_bundle`], with the settings as a struct.
pub fn create_bundle_with(
    input_dir: &Path,
    output_file: &Path,
    options: &BundleOptions,
) -> Result<()> {
    let mut bundle = BundleWriter::create(output_file, options)?;
    let base = options.file_options();

    for entry in WalkDir::new(input_dir)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry.map_err(|error| {
            let path = error.path().unwrap_or(input_dir).to_path_buf();
            Error::io(
                path,
                error
                    .into_io_error()
                    .unwrap_or_else(|| io::Error::other("could not walk the input directory")),
            )
        })?;

        let relative = match entry.path().strip_prefix(input_dir) {
            Ok(relative) if !relative.as_os_str().is_empty() => relative,
            // The root itself, which needs no entry.
            _ => continue,
        };

        let name = names::normalize(&relative.to_string_lossy())?;

        // Not followed and not stored: a link is a reference to this
        // machine, and a bundle is meant to travel.
        if entry.path_is_symlink() {
            continue;
        }

        if entry.file_type().is_dir() {
            // Kept so an empty directory survives the round trip; walkdir
            // plus `is_file()` alone would drop it.
            bundle
                .zip
                .add_directory(&name, base)
                .map_err(|error| Error::zip(output_file, &name, error))?;
            continue;
        }

        if !entry.file_type().is_file() {
            // Sockets, fifos, devices. A bundle is files and directories.
            continue;
        }

        bundle
            .zip
            .start_file(&name, base)
            .map_err(|error| Error::zip(output_file, &name, error))?;

        let source = File::open(entry.path()).map_err(|error| Error::io(entry.path(), error))?;
        // Streamed, not slurped: this is what keeps a 4 GB asset off the
        // heap on the way in.
        io::copy(
            &mut BufReader::with_capacity(64 * 1024, source),
            &mut bundle.zip,
        )
        .map_err(|error| Error::io(entry.path(), error))?;
    }

    bundle.finish()
}

/// A bundle being built entry by entry, for content that is produced rather
/// than already sitting in a directory.
///
/// [`create_bundle`] needs the files to exist first. An exporter — a FLAC
/// encoder, a video muxer — wants the opposite: somewhere to *write* that ends
/// up in the bundle. [`entry`](BundleWriter::entry) is that somewhere: an
/// [`EntryWriter`] that is [`Write`] + [`Seek`] + [`Read`], so it goes anywhere
/// an exporter would otherwise take a file.
///
/// ```no_run
/// use std::io::Write;
/// use pfac::{BundleOptions, BundleWriter};
///
/// let mut bundle = BundleWriter::create("take.pfac", &BundleOptions::stored())?;
///
/// let mut entry = bundle.entry("music/take.flac")?;
/// entry.write_all(b"fLaC")?;      // an encoder writes here instead of to a path
/// entry.finish()?;                // only now is it in the bundle
///
/// bundle.finish()?;               // only now is the bundle on disk
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// Like [`create_bundle`], the bundle is built beside `output_file` and renamed
/// into place by [`finish`](BundleWriter::finish); dropping the writer without
/// finishing leaves any previous bundle as it was and writes nothing.
pub struct BundleWriter {
    zip: ZipWriter<NamedTempFile>,
    output: PathBuf,
    options: BundleOptions,
    /// Names already in the bundle, so a repeat fails when it is opened rather
    /// than after a four-gigabyte export has been spooled.
    written: HashSet<String>,
}

impl BundleWriter {
    /// Starts a bundle at `output_file`, which is not touched until
    /// [`finish`](BundleWriter::finish).
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the directory cannot be made or written to.
    pub fn create(output_file: impl AsRef<Path>, options: &BundleOptions) -> Result<Self> {
        let output_file = output_file.as_ref();

        // The temporary has to share a filesystem with the destination, or the
        // rename becomes a copy and a copy is not atomic.
        let temp = match output_file.parent().filter(|p| !p.as_os_str().is_empty()) {
            Some(dir) => {
                std::fs::create_dir_all(dir).map_err(|error| Error::io(dir, error))?;
                NamedTempFile::new_in(dir).map_err(|error| Error::io(dir, error))?
            }
            None => NamedTempFile::new().map_err(|error| Error::io(output_file, error))?,
        };

        Ok(BundleWriter {
            zip: ZipWriter::new(temp),
            output: output_file.to_path_buf(),
            options: options.clone(),
            written: HashSet::new(),
        })
    }

    /// Opens a new entry called `name` to be written.
    ///
    /// The name is normalised as [`create_bundle`] normalises one, so
    /// `music\take.flac` is stored as `music/take.flac`.
    ///
    /// Only one entry is open at a time — the borrow sees to that — and each
    /// must be [`finish`](EntryWriter::finish)ed to land in the bundle.
    ///
    /// # Errors
    ///
    /// [`Error::BadName`] for a name that cannot be stored portably, or one
    /// already in this bundle; [`Error::Io`] if the scratch file cannot be made.
    pub fn entry(&mut self, name: &str) -> Result<EntryWriter<'_>> {
        let name = names::normalize(name)?;

        if self.written.contains(&name) {
            return Err(Error::bad_name(name, "already written to this bundle"));
        }

        // Anonymous: no name on disk, gone when it closes, so a crash leaves
        // nothing behind to clean up.
        let spool = tempfile::tempfile().map_err(|error| Error::io(&self.output, error))?;

        Ok(EntryWriter {
            bundle: self,
            name,
            spool,
        })
    }

    /// Writes the index, flushes, and renames the bundle into place.
    ///
    /// # Errors
    ///
    /// [`Error::Zip`] if the archive cannot be finished, [`Error::Io`] if it
    /// cannot be flushed or moved.
    pub fn finish(self) -> Result<()> {
        let output_file = self.output;
        let mut temp = self
            .zip
            .finish()
            .map_err(|error| Error::zip(&output_file, "", error))?;

        // Both buffers: the file's, then the kernel's. Skipping either means the
        // rename can publish a bundle whose tail is not on disk yet.
        temp.flush()
            .map_err(|error| Error::io(&output_file, error))?;
        temp.as_file()
            .sync_all()
            .map_err(|error| Error::io(&output_file, error))?;

        temp.persist(&output_file)
            .map_err(|error| Error::io(&output_file, error.error))?;

        Ok(())
    }
}

/// One entry of a [`BundleWriter`], written to as a file would be.
///
/// [`Write`] + [`Seek`] + [`Read`], so it fits exporters that patch a header
/// after the fact (FLAC's total length) or move an index to the front (MP4's
/// `moov`). A zip entry cannot do either — it is written once, forward — so the
/// bytes are held in an anonymous scratch file, and copied into the bundle by
/// [`finish`](EntryWriter::finish). The scratch file is disk, not memory: an
/// export of any size costs a copy, never a buffer the size of the export.
///
/// Unbuffered: wrap it in a [`BufWriter`](std::io::BufWriter) when writing in
/// small pieces.
///
/// **An entry that is dropped, not finished, is discarded** — nothing reaches
/// the bundle. That is deliberate: an export that failed halfway should not
/// leave half a file in there. Dropping cannot report an error, so landing the
/// entry is an explicit call.
pub struct EntryWriter<'a> {
    bundle: &'a mut BundleWriter,
    name: String,
    spool: File,
}

impl EntryWriter<'_> {
    /// The name the entry will have, as normalised.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Copies what was written into the bundle, and returns how many bytes that
    /// was.
    ///
    /// # Errors
    ///
    /// [`Error::Zip`] if the entry cannot be added, [`Error::Io`] if the scratch
    /// file cannot be read back.
    pub fn finish(mut self) -> Result<u64> {
        let output = self.bundle.output.clone();
        let io = |error| Error::io(&output, error);

        let length = self.spool.metadata().map_err(io)?.len();
        self.spool.seek(SeekFrom::Start(0)).map_err(io)?;

        let bundle = self.bundle;
        bundle
            .zip
            .start_file(&self.name, bundle.options.file_options())
            .map_err(|error| Error::zip(&bundle.output, &self.name, error))?;

        io::copy(
            &mut BufReader::with_capacity(64 * 1024, &mut self.spool).take(length),
            &mut bundle.zip,
        )
        .map_err(|error| Error::io(&bundle.output, error))?;

        bundle.written.insert(self.name);

        Ok(length)
    }
}

impl Write for EntryWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.spool.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.spool.flush()
    }
}

impl Read for EntryWriter<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.spool.read(buf)
    }
}

impl Seek for EntryWriter<'_> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.spool.seek(pos)
    }
}
