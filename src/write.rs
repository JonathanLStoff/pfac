//! Building a bundle from a directory tree.

use std::fs::File;
use std::io::{self, BufReader, Write};
use std::path::Path;

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
/// ```no_run
/// // Uncompressed, so every entry can be streamed back out.
/// pfac::create_bundle("./assets".as_ref(), "assets.pfac".as_ref(), false, None)?;
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
    // The temporary has to share a filesystem with the destination, or the
    // rename becomes a copy and a copy is not atomic.
    let mut temp = match output_file.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(dir) => {
            std::fs::create_dir_all(dir).map_err(|error| Error::io(dir, error))?;
            NamedTempFile::new_in(dir).map_err(|error| Error::io(dir, error))?
        }
        None => NamedTempFile::new().map_err(|error| Error::io(output_file, error))?,
    };

    {
        let mut zip = ZipWriter::new(&mut temp);
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
                zip.add_directory(&name, base)
                    .map_err(|error| Error::zip(output_file, &name, error))?;
                continue;
            }

            if !entry.file_type().is_file() {
                // Sockets, fifos, devices. A bundle is files and directories.
                continue;
            }

            zip.start_file(&name, base)
                .map_err(|error| Error::zip(output_file, &name, error))?;

            let source =
                File::open(entry.path()).map_err(|error| Error::io(entry.path(), error))?;
            // Streamed, not slurped: this is what keeps a 4 GB asset off the
            // heap on the way in.
            io::copy(&mut BufReader::with_capacity(64 * 1024, source), &mut zip)
                .map_err(|error| Error::io(entry.path(), error))?;
        }

        zip.finish()
            .map_err(|error| Error::zip(output_file, "", error))?;
    }

    // Both buffers: the file's, then the kernel's. Skipping either means the
    // rename can publish a bundle whose tail is not on disk yet.
    temp.flush()
        .map_err(|error| Error::io(output_file, error))?;
    temp.as_file()
        .sync_all()
        .map_err(|error| Error::io(output_file, error))?;

    temp.persist(output_file)
        .map_err(|error| Error::io(output_file, error.error))?;

    Ok(())
}
