//! The file's name is the caller's business.
//!
//! `.pfac` is a convention. Nothing in the crate reads an extension — a bundle
//! is identified by zip's own central directory — so these are all the same
//! format wearing different names.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use pfac::{Bundle, DEFAULT_EXTENSION, create_bundle, with_default_extension};
use tempfile::TempDir;

fn input(temp: &TempDir) -> PathBuf {
    let root = temp.path().join("input");
    fs::create_dir_all(root.join("nested")).unwrap();
    fs::write(root.join("readme.txt"), b"contents").unwrap();
    fs::write(root.join("nested").join("deep.bin"), b"deeper").unwrap();
    root
}

/// Writes a bundle at `name` and reads an entry back out of it.
fn round_trip(temp: &TempDir, root: &Path, name: &str) {
    let output = temp.path().join(name);

    create_bundle(root, &output, false, None).unwrap();
    assert!(output.is_file(), "{name} was not written");

    let mut bundle = Bundle::open(&output).unwrap();
    assert_eq!(bundle.read("readme.txt").unwrap(), b"contents");
    assert_eq!(bundle.read("nested/deep.bin").unwrap(), b"deeper");

    let mut stream = bundle.stream("readme.txt").unwrap();
    let mut streamed = Vec::new();
    stream.read_to_end(&mut streamed).unwrap();
    assert_eq!(streamed, b"contents");
}

#[test]
fn any_extension_works() {
    let temp = TempDir::new().unwrap();
    let root = input(&temp);

    for name in [
        "assets.pfac",
        "assets.zip",
        "assets.bundle",
        "level-01.myapp-project",
        "assets.PFAC",
        "assets.7z",
        // Deliberately misleading. The name is not consulted, so it opens.
        "assets.tar.gz",
        // Dots in the stem rather than a meaningful extension.
        "v1.2.3",
    ] {
        round_trip(&temp, &root, name);
    }
}

#[test]
fn no_extension_at_all_works() {
    let temp = TempDir::new().unwrap();
    let root = input(&temp);

    round_trip(&temp, &root, "assets");
}

#[test]
fn renaming_a_bundle_does_not_stop_it_opening() {
    let temp = TempDir::new().unwrap();
    let root = input(&temp);

    let written = temp.path().join("original.pfac");
    create_bundle(&root, &written, false, None).unwrap();

    let renamed = temp.path().join("renamed.somethingelse");
    fs::rename(&written, &renamed).unwrap();

    let mut bundle = Bundle::open(&renamed).unwrap();
    assert_eq!(bundle.read("readme.txt").unwrap(), b"contents");
}

#[test]
fn a_bundle_in_a_dotted_directory_is_fine() {
    let temp = TempDir::new().unwrap();
    let root = input(&temp);

    let nested = temp.path().join("out.v2").join("build.1");
    let output = nested.join("assets.pfac");

    // The parent directories are created on the way.
    create_bundle(&root, &output, false, None).unwrap();

    assert!(Bundle::open(&output).is_ok());
}

#[test]
fn the_default_extension_is_pfac_and_only_a_default() {
    assert_eq!(DEFAULT_EXTENSION, "pfac");

    // Applied only where there is nothing already.
    assert_eq!(with_default_extension("assets"), Path::new("assets.pfac"));
    assert_eq!(
        with_default_extension("some/dir/assets"),
        Path::new("some/dir/assets.pfac")
    );

    // Never overrides a choice the caller already made.
    for already in ["assets.zip", "assets.pfac", "level.myapp", "v1.2"] {
        assert_eq!(with_default_extension(already), Path::new(already));
    }
}

#[test]
fn the_helper_and_a_hand_written_name_agree() {
    let temp = TempDir::new().unwrap();
    let root = input(&temp);

    let helped = with_default_extension(temp.path().join("a"));
    let literal = temp.path().join("b.pfac");

    create_bundle(&root, &helped, false, None).unwrap();
    create_bundle(&root, &literal, false, None).unwrap();

    assert_eq!(helped.extension().unwrap(), "pfac");
    assert_eq!(
        Bundle::open(&helped).unwrap().len(),
        Bundle::open(&literal).unwrap().len()
    );
}

#[test]
fn a_zip_made_elsewhere_opens_whatever_it_is_called() {
    let temp = TempDir::new().unwrap();
    let root = input(&temp);

    // Written as `.zip`, opened as a bundle: same format, so this is not a
    // conversion — it is the point.
    let output = temp.path().join("made-elsewhere.zip");
    create_bundle(&root, &output, true, None).unwrap();

    let mut bundle = Bundle::open(&output).unwrap();
    assert_eq!(bundle.read("readme.txt").unwrap(), b"contents");
}
