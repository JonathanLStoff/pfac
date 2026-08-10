//! One error enum for the whole crate.
//!
//! The variants are the cases a caller can do something about. "This entry is
//! compressed, so it cannot be streamed" and "this entry does not exist" lead to
//! different fixes, so they are different variants rather than one string.

use std::path::PathBuf;

/// Anything that can go wrong writing or reading a bundle.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// No entry by that name.
    #[error("{archive}: no entry named {name:?}")]
    NoSuchEntry {
        /// The bundle that was searched.
        archive: PathBuf,
        /// The name that was looked for.
        name: String,
    },

    /// The entry was asked for as a stream, but it is compressed.
    ///
    /// Streaming needs [`Seek`](std::io::Seek), and a deflated entry can only be
    /// read forward from its start — seeking backwards would mean re-inflating
    /// from the beginning. Write the bundle with `compress: false`, or read this
    /// entry with [`Bundle::read`](crate::Bundle::read) instead.
    #[error(
        "{name:?} is {method}-compressed and cannot be streamed; \
         write the bundle uncompressed, or use `read` to take it into memory"
    )]
    Compressed {
        /// The entry.
        name: String,
        /// How it is stored, for the message.
        method: String,
    },

    /// The entry was asked for as a stream, but it is encrypted.
    ///
    /// AES entries verify their HMAC only once the whole entry has been read, so
    /// a stream would be handing out bytes that have not been authenticated yet.
    /// [`Bundle::read`](crate::Bundle::read) reads to the end and so can check
    /// it.
    #[error("{name:?} is encrypted and cannot be streamed; use `read` instead")]
    EncryptedStream {
        /// The entry.
        name: String,
    },

    /// The entry is a directory, not a file.
    #[error("{name:?} is a directory")]
    IsDirectory {
        /// The entry.
        name: String,
    },

    /// No password given for an encrypted bundle, or the wrong one.
    #[error("{archive}: wrong password, or none given")]
    Password {
        /// The bundle.
        archive: PathBuf,
    },

    /// An entry name that cannot be stored, or cannot be restored safely.
    ///
    /// Names are checked on the way in so a bundle written on one platform can
    /// be opened on another, and on the way out so a hostile bundle cannot write
    /// outside the directory it is being read into.
    #[error("{name:?} is not a usable entry name: {reason}")]
    BadName {
        /// The name as it was given.
        name: String,
        /// Why it cannot be used.
        reason: String,
    },

    /// Reading the whole entry into memory would exceed the caller's limit.
    ///
    /// A bundle's index is written by whoever built it, so an entry can claim
    /// any size at all. Without a ceiling, reading one is an out-of-memory abort
    /// rather than an error.
    #[error("{name:?} is {size} bytes, over the {limit} byte limit for reading into memory")]
    TooLarge {
        /// The entry.
        name: String,
        /// What it claims to be, or how much was actually read.
        size: u64,
        /// The ceiling it went past.
        limit: u64,
    },

    /// Filesystem trouble, with the path that caused it.
    ///
    /// [`std::io::Error`] alone does not say which file it was about, and by the
    /// time it reaches a user that is the only part they need.
    #[error("{path}: {source}")]
    Io {
        /// The file or directory the call was about.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },

    /// Trouble from the zip layer: a malformed archive, an unsupported feature.
    #[error("{path}: {source}")]
    Zip {
        /// The bundle.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: zip::result::ZipError,
    },
}

impl Error {
    /// Attaches a path to an [`std::io::Error`].
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    /// Attaches a path to a [`zip::result::ZipError`], turning the two cases a
    /// caller can act on into their own variants on the way.
    pub(crate) fn zip(path: impl Into<PathBuf>, name: &str, source: zip::result::ZipError) -> Self {
        let path = path.into();

        match source {
            zip::result::ZipError::FileNotFound => Error::NoSuchEntry {
                archive: path,
                name: name.to_string(),
            },
            zip::result::ZipError::InvalidPassword => Error::Password { archive: path },
            source => Error::Zip { path, source },
        }
    }

    pub(crate) fn bad_name(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Error::BadName {
            name: name.into(),
            reason: reason.into(),
        }
    }
}

/// The crate's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;
