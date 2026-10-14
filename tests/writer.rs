//! Writing a bundle entry by entry, through `Write + Seek`, and reading it back.

use std::io::{BufWriter, Read, Seek, SeekFrom, Write};

use pfac::{Bundle, BundleOptions, BundleWriter, Error};
use tempfile::TempDir;

/// Non-repeating enough that an off-by-one in the copy shows up.
fn blob(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[test]
fn an_entry_written_and_finished_is_in_the_bundle() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("out.pfac");
    let data = blob(300_000);

    let mut bundle = BundleWriter::create(&archive, &BundleOptions::stored()).unwrap();
    let mut entry = bundle.entry("music/take.flac").unwrap();
    entry.write_all(&data).unwrap();
    assert_eq!(entry.finish().unwrap(), data.len() as u64);
    bundle.finish().unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    assert_eq!(bundle.read("music/take.flac").unwrap(), data);

    // Stored, so it streams: the point of writing into a bundle at all.
    let mut stream = bundle.stream("music/take.flac").unwrap();
    stream.seek(SeekFrom::Start(1000)).unwrap();
    let mut window = [0_u8; 64];
    stream.read_exact(&mut window).unwrap();
    assert_eq!(window, data[1000..1064]);
}

#[test]
fn a_header_patched_after_the_fact_is_what_lands() {
    // What a FLAC or WebM writer does: a placeholder up front, the truth at the
    // end. A forward-only zip entry could not do this; the spool can.
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("out.pfac");

    let mut bundle = BundleWriter::create(&archive, &BundleOptions::stored()).unwrap();
    let mut entry = bundle.entry("patched.bin").unwrap();
    entry.write_all(&[0; 8]).unwrap();
    entry.write_all(b"body").unwrap();
    entry.seek(SeekFrom::Start(0)).unwrap();
    entry.write_all(b"HEADER!!").unwrap();
    entry.seek(SeekFrom::End(0)).unwrap();
    entry.finish().unwrap();
    bundle.finish().unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    assert_eq!(bundle.read("patched.bin").unwrap(), b"HEADER!!body");
}

#[test]
fn an_entry_can_be_read_back_before_it_is_finished() {
    let temp = TempDir::new().unwrap();
    let mut bundle =
        BundleWriter::create(temp.path().join("out.pfac"), &BundleOptions::stored()).unwrap();

    let mut entry = bundle.entry("a.bin").unwrap();
    entry.write_all(b"hello world").unwrap();
    entry.seek(SeekFrom::Start(6)).unwrap();

    let mut rest = String::new();
    entry.read_to_string(&mut rest).unwrap();
    assert_eq!(rest, "world");
}

#[test]
fn an_unfinished_entry_is_discarded() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("out.pfac");

    let mut bundle = BundleWriter::create(&archive, &BundleOptions::stored()).unwrap();

    let mut failed = bundle.entry("half.bin").unwrap();
    failed.write_all(b"half an export").unwrap();
    drop(failed);

    // The name is free again, because nothing was written under it.
    let mut kept = bundle.entry("half.bin").unwrap();
    kept.write_all(b"whole").unwrap();
    kept.finish().unwrap();
    bundle.finish().unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    assert_eq!(bundle.read("half.bin").unwrap(), b"whole");
}

#[test]
fn a_name_cannot_be_written_twice() {
    let temp = TempDir::new().unwrap();
    let mut bundle =
        BundleWriter::create(temp.path().join("out.pfac"), &BundleOptions::stored()).unwrap();

    let mut entry = bundle.entry("a.bin").unwrap();
    entry.write_all(b"one").unwrap();
    entry.finish().unwrap();

    // Refused when opened, not after a large export has been spooled.
    assert!(matches!(bundle.entry("a.bin"), Err(Error::BadName { .. })));
    // Normalised first, so a different spelling of the same name is caught.
    assert!(matches!(bundle.entry("./a.bin"), Err(Error::BadName { .. })));
    assert!(matches!(bundle.entry("a\\\\b/../x"), Err(Error::BadName { .. })));
}

#[test]
fn names_are_normalised_and_checked() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("out.pfac");
    let mut bundle = BundleWriter::create(&archive, &BundleOptions::stored()).unwrap();

    assert!(matches!(bundle.entry("../escape"), Err(Error::BadName { .. })));
    assert!(matches!(bundle.entry("/absolute"), Err(Error::BadName { .. })));
    assert!(matches!(bundle.entry(""), Err(Error::BadName { .. })));

    let mut entry = bundle.entry("music\\take.flac").unwrap();
    assert_eq!(entry.name(), "music/take.flac");
    entry.write_all(b"x").unwrap();
    entry.finish().unwrap();
    bundle.finish().unwrap();

    assert!(Bundle::open(&archive).unwrap().contains("music/take.flac"));
}

#[test]
fn compressed_and_encrypted_bundles_take_written_entries_too() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("out.pfac");
    let data = blob(100_000);

    let options = BundleOptions::compressed().with_password("hunter2");
    let mut bundle = BundleWriter::create(&archive, &options).unwrap();
    let mut entry = bundle.entry("secret.bin").unwrap();
    entry.write_all(&data).unwrap();
    entry.finish().unwrap();
    bundle.finish().unwrap();

    let mut bundle = Bundle::open_with_password(&archive, "hunter2").unwrap();
    assert_eq!(bundle.read("secret.bin").unwrap(), data);
}

#[test]
fn a_buffered_writer_over_an_entry_works() {
    // How an exporter that makes many small writes would be handed one.
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("out.pfac");

    let mut bundle = BundleWriter::create(&archive, &BundleOptions::stored()).unwrap();
    let mut entry = bundle.entry("small.bin").unwrap();
    {
        let mut buffered = BufWriter::new(&mut entry);
        for byte in 0..=255_u8 {
            buffered.write_all(&[byte]).unwrap();
        }
        buffered.flush().unwrap();
    }
    assert_eq!(entry.finish().unwrap(), 256);
    bundle.finish().unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    assert_eq!(bundle.read("small.bin").unwrap(), (0..=255_u8).collect::<Vec<_>>());
}

#[test]
fn an_unfinished_bundle_leaves_nothing_where_it_was_going() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("out.pfac");

    let mut bundle = BundleWriter::create(&archive, &BundleOptions::stored()).unwrap();
    let mut entry = bundle.entry("a.bin").unwrap();
    entry.write_all(b"x").unwrap();
    entry.finish().unwrap();
    drop(bundle);

    assert!(!archive.exists());
}

#[test]
fn an_update_replaces_one_entry_and_carries_the_rest_over_untouched() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("out.pfac");
    let options = BundleOptions::stored().with_password("hunter2");

    let mut bundle = BundleWriter::create(&archive, &options).unwrap();
    for (name, data) in [("keep.bin", blob(5_000)), ("core.planet", b"old".to_vec())] {
        let mut entry = bundle.entry(name).unwrap();
        entry.write_all(&data).unwrap();
        entry.finish().unwrap();
    }
    bundle.finish().unwrap();

    let mut bundle = BundleWriter::update(&archive, &options).unwrap();
    let mut entry = bundle.entry("core.planet").unwrap();
    entry.write_all(b"new and longer").unwrap();
    entry.finish().unwrap();
    let mut entry = bundle.entry("added.bin").unwrap();
    entry.write_all(b"added").unwrap();
    entry.finish().unwrap();
    bundle.finish().unwrap();

    let mut bundle = Bundle::open_with_password(&archive, "hunter2").unwrap();
    let mut names: Vec<&str> = bundle.names().collect();
    names.sort_unstable();
    assert_eq!(names, ["added.bin", "core.planet", "keep.bin"], "no entry twice");

    assert_eq!(bundle.read("core.planet").unwrap(), b"new and longer");
    assert_eq!(bundle.read("added.bin").unwrap(), b"added");
    assert_eq!(bundle.read("keep.bin").unwrap(), blob(5_000), "decrypted still, never re-encrypted");
}

#[test]
fn a_bundle_can_be_updated_while_an_entry_is_being_streamed_from_it() {
    // An app plays its assets straight out of the bundle and saves into it at
    // the same time. On Windows, replacing a file someone has open was refused
    // (os error 5) until the rename went through POSIX semantics.
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("out.pfac");
    let asset = blob(50_000);

    let mut bundle = BundleWriter::create(&archive, &BundleOptions::stored()).unwrap();
    let mut entry = bundle.entry("asset.flac").unwrap();
    entry.write_all(&asset).unwrap();
    entry.finish().unwrap();
    bundle.finish().unwrap();

    let mut playing = Bundle::open(&archive).unwrap().stream("asset.flac").unwrap();

    let mut bundle = BundleWriter::update(&archive, &BundleOptions::stored()).unwrap();
    let mut entry = bundle.entry("core.planet").unwrap();
    entry.write_all(b"saved").unwrap();
    entry.finish().unwrap();
    bundle.finish().unwrap();

    // The open stream still reads the bundle it opened.
    let mut heard = Vec::new();
    playing.read_to_end(&mut heard).unwrap();
    assert_eq!(heard, asset);

    let mut bundle = Bundle::open(&archive).unwrap();
    assert_eq!(bundle.read("core.planet").unwrap(), b"saved");
    assert_eq!(bundle.read("asset.flac").unwrap(), asset);

    // Published, so nothing of the temporary is left beside it.
    let left: Vec<_> = std::fs::read_dir(temp.path()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert_eq!(left, ["out.pfac"]);
}

#[test]
fn an_unfinished_update_leaves_the_original_as_it_was() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("out.pfac");

    let mut bundle = BundleWriter::create(&archive, &BundleOptions::stored()).unwrap();
    let mut entry = bundle.entry("a.bin").unwrap();
    entry.write_all(b"original").unwrap();
    entry.finish().unwrap();
    bundle.finish().unwrap();

    let mut bundle = BundleWriter::update(&archive, &BundleOptions::stored()).unwrap();
    let mut entry = bundle.entry("a.bin").unwrap();
    entry.write_all(b"half an export").unwrap();
    entry.finish().unwrap();
    drop(bundle);

    let mut bundle = Bundle::open(&archive).unwrap();
    assert_eq!(bundle.read("a.bin").unwrap(), b"original");
}

#[test]
fn updating_a_bundle_that_is_not_there_fails_at_once() {
    let temp = TempDir::new().unwrap();
    let missing = temp.path().join("nope.pfac");

    assert!(matches!(
        BundleWriter::update(&missing, &BundleOptions::stored()),
        Err(Error::Io { .. })
    ));
}

#[test]
fn an_update_keeps_directories_and_compressed_entries() {
    let temp = TempDir::new().unwrap();
    let input = temp.path().join("input");
    std::fs::create_dir_all(input.join("empty")).unwrap();
    std::fs::write(input.join("text.txt"), "hello ".repeat(1000)).unwrap();
    let archive = temp.path().join("out.pfac");
    pfac::create_bundle(&input, &archive, true, None).unwrap();

    let mut bundle = BundleWriter::update(&archive, &BundleOptions::compressed()).unwrap();
    let mut entry = bundle.entry("new.txt").unwrap();
    entry.write_all(b"new").unwrap();
    entry.finish().unwrap();
    bundle.finish().unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    assert!(bundle.names().any(|name| name == "empty/"));
    assert_eq!(bundle.read("text.txt").unwrap(), "hello ".repeat(1000).into_bytes());
    assert_eq!(bundle.read("new.txt").unwrap(), b"new");
}

#[test]
fn an_entry_can_be_removed_from_a_bundle_being_updated() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("out.pfac");

    let mut bundle = BundleWriter::create(&archive, &BundleOptions::stored()).unwrap();
    for name in ["keep.bin", "asset/audio/old.flac"] {
        let mut entry = bundle.entry(name).unwrap();
        entry.write_all(b"x").unwrap();
        entry.finish().unwrap();
    }
    bundle.finish().unwrap();

    let mut bundle = BundleWriter::update(&archive, &BundleOptions::stored()).unwrap();
    assert!(bundle.remove("asset/audio/old.flac").unwrap());
    assert!(!bundle.remove("never/there.bin").unwrap(), "already gone is not an error");
    bundle.finish().unwrap();

    let bundle = Bundle::open(&archive).unwrap();
    assert!(bundle.contains("keep.bin"));
    assert!(!bundle.contains("asset/audio/old.flac"));
}

#[test]
fn a_removal_that_is_never_finished_changes_nothing() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("out.pfac");

    let mut bundle = BundleWriter::create(&archive, &BundleOptions::stored()).unwrap();
    let mut entry = bundle.entry("a.bin").unwrap();
    entry.write_all(b"x").unwrap();
    entry.finish().unwrap();
    bundle.finish().unwrap();

    let mut bundle = BundleWriter::update(&archive, &BundleOptions::stored()).unwrap();
    bundle.remove("a.bin").unwrap();
    drop(bundle);

    assert!(Bundle::open(&archive).unwrap().contains("a.bin"));
}

#[test]
fn removing_and_rewriting_one_name_leaves_the_new_one() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("out.pfac");

    let mut bundle = BundleWriter::create(&archive, &BundleOptions::stored()).unwrap();
    let mut entry = bundle.entry("a.bin").unwrap();
    entry.write_all(b"old").unwrap();
    entry.finish().unwrap();
    bundle.finish().unwrap();

    let mut bundle = BundleWriter::update(&archive, &BundleOptions::stored()).unwrap();
    bundle.remove("a.bin").unwrap();
    let mut entry = bundle.entry("a.bin").unwrap();
    entry.write_all(b"new").unwrap();
    entry.finish().unwrap();
    bundle.finish().unwrap();

    assert_eq!(Bundle::open(&archive).unwrap().read("a.bin").unwrap(), b"new");
}

#[test]
fn an_entry_already_written_cannot_be_removed() {
    let temp = TempDir::new().unwrap();
    let mut bundle =
        BundleWriter::create(temp.path().join("out.pfac"), &BundleOptions::stored()).unwrap();
    let mut entry = bundle.entry("a.bin").unwrap();
    entry.write_all(b"x").unwrap();
    entry.finish().unwrap();

    assert!(matches!(bundle.remove("a.bin"), Err(Error::BadName { .. })));
}
