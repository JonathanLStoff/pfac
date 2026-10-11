//! Adds a file to an asset folder of a bundle that already exists, and replaces
//! another, without unpacking anything.
//!
//! ```sh
//! make example-update                      # -> target/example/project.pfac
//! make example-update OUT=my_project.pfac  # an existing bundle of your own
//! ```
//!
//! With no bundle at the path, a small project is made first so there is
//! something to update. With one, it is updated in place — the new entries go
//! under `assets/audio/` whether or not the folder was there.
//!
//! `BundleWriter::update` is the whole trick. What you write is added, or
//! replaces the entry of the same name; every other entry is carried over
//! as stored, so nothing is decompressed or decrypted on the way through. The
//! bundle is rebuilt beside the original and renamed over it, so a failure
//! halfway leaves the original as it was.

use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;

use pfac::{Bundle, BundleOptions, BundleWriter, create_bundle, with_default_extension};

fn main() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bundle_path = with_default_extension(
        env::args()
            .nth(1)
            .filter(|argument| !argument.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("target/example/project")),
    );

    // --- an existing bundle ----------------------------------------------

    if !bundle_path.exists() {
        make_demo_project(&bundle_path)?;
    }

    println!("before:");
    list(&bundle_path)?;

    // --- update it --------------------------------------------------------

    // Stored, like the rest of the demo. For an encrypted bundle this is where
    // `.with_password(...)` goes: options apply to what is written here, and
    // the untouched entries keep the encryption they have.
    let mut bundle = BundleWriter::update(&bundle_path, &BundleOptions::stored())?;

    // A file from disk, into a folder inside the bundle. `entry` is a writer, so
    // `io::copy` streams it in; nothing is held whole.
    let source = root.join("tests/data/output_440hz_5s.flac");
    let mut entry = bundle.entry("assets/audio/take-2.flac")?;
    io::copy(&mut File::open(&source)?, &mut entry)?;
    let bytes = entry.finish()?;
    println!("\nadded    assets/audio/take-2.flac ({bytes} bytes, from {})", source.display());

    // Produced content, no file at all.
    let mut entry = bundle.entry("assets/audio/notes.txt")?;
    writeln!(entry, "take 2 was added to this bundle after it was made")?;
    entry.finish()?;
    println!("added    assets/audio/notes.txt");

    // An entry that already exists is replaced, not duplicated.
    let mut entry = bundle.entry("assets/images/cover.txt")?;
    entry.write_all(b"cover, second edition")?;
    entry.finish()?;
    println!("replaced assets/images/cover.txt");

    // Nothing has touched the bundle on disk until this.
    bundle.finish()?;

    // --- read it back -----------------------------------------------------

    println!("\nafter:");
    list(&bundle_path)?;

    let mut bundle = Bundle::open(&bundle_path)?;
    assert_eq!(bundle.read("assets/images/cover.txt")?, b"cover, second edition");
    assert_eq!(bundle.read("assets/audio/click.txt")?, b"the original click", "untouched");
    assert_eq!(bundle.names().filter(|name| *name == "assets/images/cover.txt").count(), 1);

    println!("\nthe new file in assets/audio/ streams like any other:");
    let stream = bundle.stream("assets/audio/take-2.flac")?;
    println!("  assets/audio/take-2.flac: {} bytes, seekable", stream.len());

    Ok(())
}

/// A project to update: two asset folders and a database, the shape OrbitX uses.
fn make_demo_project(bundle_path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    let source = bundle_path.with_extension("src");
    let _ = fs::remove_dir_all(&source);

    fs::create_dir_all(source.join("assets/audio"))?;
    fs::create_dir_all(source.join("assets/images"))?;
    fs::write(source.join("assets/audio/click.txt"), b"the original click")?;
    fs::write(source.join("assets/images/cover.txt"), b"cover, first edition")?;
    fs::write(source.join("core.db"), b"not really a database")?;

    create_bundle(&source, bundle_path, false, None)?;
    fs::remove_dir_all(&source)?;

    println!("no bundle at {}; made a demo project", bundle_path.display());
    Ok(())
}

fn list(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    let bundle = Bundle::open(path)?;
    let mut names: Vec<&str> = bundle.names().collect();
    names.sort_unstable();

    for name in names {
        println!("  {name}");
    }
    Ok(())
}
