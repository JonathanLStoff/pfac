//! Writing a bundle and reading it back.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use pfac::{Bundle, BundleOptions, Error, create_bundle, create_bundle_with};
use tempfile::TempDir;

/// A tree with subfolders at two depths, an empty directory, and a file big
/// enough that a partial read is meaningful.
fn tree() -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("input");

    write(&root.join("readme.txt"), b"top level");
    write(&root.join("music").join("take.flac"), &blob(200_000));
    write(&root.join("art").join("cover.png"), b"\x89PNG cover");
    write(
        &root.join("art").join("icons").join("play.png"),
        b"\x89PNG play",
    );
    fs::create_dir_all(root.join("empty")).unwrap();

    (temp, root)
}

fn write(path: &Path, contents: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Non-repeating enough that an off-by-one in the windowing shows up.
fn blob(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[test]
fn subfolders_survive_at_any_depth() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");

    create_bundle(&root, &archive, false, None).unwrap();

    let bundle = Bundle::open(&archive).unwrap();
    let names: Vec<&str> = bundle.names().collect();

    for expected in [
        "readme.txt",
        "music/take.flac",
        "art/cover.png",
        "art/icons/play.png",
    ] {
        assert!(
            names.contains(&expected),
            "{expected} missing from {names:?}"
        );
    }
}

#[test]
fn an_empty_directory_survives() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");

    create_bundle(&root, &archive, false, None).unwrap();

    let bundle = Bundle::open(&archive).unwrap();
    assert!(
        bundle.names().any(|name| name.starts_with("empty")),
        "the empty directory was dropped"
    );
}

#[test]
fn subfolder_contents_can_be_listed() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");
    create_bundle(&root, &archive, false, None).unwrap();

    let bundle = Bundle::open(&archive).unwrap();
    let under: Vec<&str> = bundle.names_under("art/").collect();

    assert!(under.contains(&"art/cover.png"));
    assert!(under.contains(&"art/icons/play.png"));
    assert!(!under.contains(&"readme.txt"));
}

#[test]
fn a_streamed_entry_matches_the_original() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");
    create_bundle(&root, &archive, false, None).unwrap();

    let original = fs::read(root.join("music").join("take.flac")).unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    let mut stream = bundle.stream("music/take.flac").unwrap();

    assert_eq!(stream.len(), original.len() as u64);

    let mut streamed = Vec::new();
    stream.read_to_end(&mut streamed).unwrap();

    assert_eq!(streamed, original);
}

#[test]
fn a_stream_seeks_like_a_file() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");
    create_bundle(&root, &archive, false, None).unwrap();

    let original = fs::read(root.join("music").join("take.flac")).unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    let mut stream = bundle.stream("music/take.flac").unwrap();

    // Jump into the middle, as a decoder resolving a seek point would.
    stream.seek(SeekFrom::Start(100_000)).unwrap();
    let mut middle = [0_u8; 64];
    stream.read_exact(&mut middle).unwrap();
    assert_eq!(&middle[..], &original[100_000..100_064]);

    // Backwards, which is the thing a deflate stream cannot do.
    stream.seek(SeekFrom::Start(16)).unwrap();
    let mut early = [0_u8; 64];
    stream.read_exact(&mut early).unwrap();
    assert_eq!(&early[..], &original[16..80]);

    // Relative to the end.
    stream.seek(SeekFrom::End(-8)).unwrap();
    let mut tail = Vec::new();
    stream.read_to_end(&mut tail).unwrap();
    assert_eq!(tail, &original[original.len() - 8..]);
}

#[test]
fn a_stream_cannot_read_past_its_own_entry() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");
    create_bundle(&root, &archive, false, None).unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    let mut stream = bundle.stream("art/cover.png").unwrap();

    let mut everything = Vec::new();
    stream.read_to_end(&mut everything).unwrap();

    assert_eq!(everything, b"\x89PNG cover");
}

/// The case `ZipFile<'a>` cannot express, and the reason `EntryReader` carries
/// its own handle.
#[test]
fn two_streams_can_be_open_at_once() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");
    create_bundle(&root, &archive, false, None).unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    let mut song = bundle.stream("music/take.flac").unwrap();
    let mut cover = bundle.stream("art/cover.png").unwrap();

    // Interleaved, to be sure neither is disturbing the other's position.
    let mut song_head = [0_u8; 4];
    song.read_exact(&mut song_head).unwrap();

    let mut cover_all = Vec::new();
    cover.read_to_end(&mut cover_all).unwrap();

    let mut song_next = [0_u8; 4];
    song.read_exact(&mut song_next).unwrap();

    assert_eq!(cover_all, b"\x89PNG cover");
    assert_eq!(song_head, [0, 1, 2, 3]);
    assert_eq!(song_next, [4, 5, 6, 7]);
}

#[test]
fn a_stream_stays_usable_while_other_entries_are_read() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");
    create_bundle(&root, &archive, false, None).unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    let mut song = bundle.stream("music/take.flac").unwrap();

    let mut before = [0_u8; 4];
    song.read_exact(&mut before).unwrap();

    // The bundle is borrowed again here; the stream must not care.
    let cover = bundle.read("art/cover.png").unwrap();
    assert_eq!(cover, b"\x89PNG cover");

    let mut after = [0_u8; 4];
    song.read_exact(&mut after).unwrap();
    assert_eq!(after, [4, 5, 6, 7]);
}

#[test]
fn a_stream_can_be_moved_to_another_thread() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");
    create_bundle(&root, &archive, false, None).unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    let mut stream = bundle.stream("music/take.flac").unwrap();

    // What an audio decoder on its own thread would need.
    let handle = std::thread::spawn(move || {
        let mut head = [0_u8; 8];
        stream.read_exact(&mut head).unwrap();
        head
    });

    assert_eq!(handle.join().unwrap(), [0, 1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn streaming_a_compressed_entry_is_refused_with_advice() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");
    create_bundle(&root, &archive, true, None).unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    let result = bundle.stream("music/take.flac");

    let Err(Error::Compressed { name, .. }) = result else {
        panic!("expected Compressed, got {result:?}");
    };
    assert_eq!(name, "music/take.flac");

    // And the message points at the fix rather than just failing.
    let message = Error::Compressed {
        name: name.clone(),
        method: "deflate".to_string(),
    }
    .to_string();
    assert!(message.contains("uncompressed"), "unhelpful: {message}");
}

#[test]
fn a_compressed_entry_still_reads_into_memory() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");
    create_bundle(&root, &archive, true, None).unwrap();

    let original = fs::read(root.join("music").join("take.flac")).unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    assert_eq!(bundle.read("music/take.flac").unwrap(), original);
}

#[test]
fn compression_actually_compresses() {
    let (temp, root) = tree();
    let stored = temp.path().join("stored.pfac");
    let deflated = temp.path().join("deflated.pfac");

    create_bundle(&root, &stored, false, None).unwrap();
    create_bundle(&root, &deflated, true, None).unwrap();

    // The blob is a repeating byte pattern, so deflate should win easily.
    assert!(
        deflated.metadata().unwrap().len() < stored.metadata().unwrap().len(),
        "compressed bundle was not smaller"
    );
}

#[test]
fn an_encrypted_entry_reads_with_the_password_and_not_without() {
    let (temp, root) = tree();
    let archive = temp.path().join("secret.pfac");
    create_bundle(&root, &archive, false, Some("hunter2")).unwrap();

    let original = fs::read(root.join("art").join("cover.png")).unwrap();

    let mut right = Bundle::open_with_password(&archive, "hunter2").unwrap();
    assert_eq!(right.read("art/cover.png").unwrap(), original);

    let mut wrong = Bundle::open_with_password(&archive, "hunter3").unwrap();
    assert!(
        matches!(wrong.read("art/cover.png"), Err(Error::Password { .. })),
        "a wrong password should be its own error"
    );

    let mut none = Bundle::open(&archive).unwrap();
    assert!(none.read("art/cover.png").is_err());
}

#[test]
fn streaming_an_encrypted_entry_is_refused() {
    let (temp, root) = tree();
    let archive = temp.path().join("secret.pfac");
    create_bundle(&root, &archive, false, Some("hunter2")).unwrap();

    let mut bundle = Bundle::open_with_password(&archive, "hunter2").unwrap();

    assert!(matches!(
        bundle.stream("art/cover.png"),
        Err(Error::EncryptedStream { .. })
    ));
}

#[test]
fn entry_names_are_readable_without_the_password() {
    let (temp, root) = tree();
    let archive = temp.path().join("secret.pfac");
    create_bundle(&root, &archive, false, Some("hunter2")).unwrap();

    // Documented, not accidental: zip encrypts contents and not the index.
    // Asserted so that if it ever stops being true, the docs get revisited.
    let bundle = Bundle::open(&archive).unwrap();
    assert!(bundle.names().any(|name| name == "art/cover.png"));
}

#[test]
fn reading_is_capped_so_a_huge_entry_is_an_error_not_an_abort() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");
    create_bundle(&root, &archive, false, None).unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    bundle.set_max_read(1024);

    let result = bundle.read("music/take.flac");
    let Err(Error::TooLarge { size, limit, .. }) = result else {
        panic!("expected TooLarge, got {result:?}");
    };
    assert_eq!(limit, 1024);
    assert_eq!(size, 200_000);

    // Small entries are unaffected.
    assert!(bundle.read("art/cover.png").is_ok());
}

#[test]
fn info_reports_what_can_be_streamed() {
    let (temp, root) = tree();
    let stored = temp.path().join("stored.pfac");
    let deflated = temp.path().join("deflated.pfac");
    create_bundle(&root, &stored, false, None).unwrap();
    create_bundle(&root, &deflated, true, None).unwrap();

    let mut open = Bundle::open(&stored).unwrap();
    let info = open.info("music/take.flac").unwrap();
    assert!(info.stored);
    assert!(!info.encrypted);
    assert!(info.streamable());
    assert_eq!(info.size, 200_000);

    let mut packed = Bundle::open(&deflated).unwrap();
    let info = packed.info("music/take.flac").unwrap();
    assert!(!info.stored);
    assert!(!info.streamable());
    // The uncompressed size is still what the entry is worth.
    assert_eq!(info.size, 200_000);
}

#[test]
fn a_missing_entry_says_so() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");
    create_bundle(&root, &archive, false, None).unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();

    assert!(!bundle.contains("nope.txt"));
    assert!(matches!(
        bundle.stream("nope.txt"),
        Err(Error::NoSuchEntry { .. })
    ));
    assert!(matches!(
        bundle.read("nope.txt"),
        Err(Error::NoSuchEntry { .. })
    ));
}

#[test]
fn a_directory_is_not_a_file() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");
    create_bundle(&root, &archive, false, None).unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    let directory: String = bundle
        .names()
        .find(|name| name.ends_with('/'))
        .unwrap()
        .to_string();

    assert!(matches!(
        bundle.stream(&directory),
        Err(Error::IsDirectory { .. })
    ));
}

#[test]
fn a_file_that_is_not_a_zip_is_refused() {
    let temp = TempDir::new().unwrap();
    let not_ours = temp.path().join("notes.txt");
    fs::write(&not_ours, b"just some text").unwrap();

    assert!(Bundle::open(&not_ours).is_err());
}

#[test]
fn extract_puts_the_tree_back() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");
    let out = temp.path().join("out");
    create_bundle(&root, &archive, true, None).unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    bundle.extract(&out).unwrap();

    assert_eq!(
        fs::read(out.join("music").join("take.flac")).unwrap(),
        fs::read(root.join("music").join("take.flac")).unwrap()
    );
    assert!(out.join("art").join("icons").join("play.png").is_file());
    assert!(out.join("empty").is_dir());
}

#[test]
fn an_interrupted_write_does_not_destroy_the_previous_bundle() {
    let (temp, root) = tree();
    let archive = temp.path().join("assets.pfac");
    create_bundle(&root, &archive, false, None).unwrap();
    let good = fs::read(&archive).unwrap();

    // A source directory that does not exist fails partway through the write.
    let missing = temp.path().join("gone");
    let result = create_bundle(&missing, &archive, false, None);

    if result.is_err() {
        assert_eq!(
            fs::read(&archive).unwrap(),
            good,
            "a failed write damaged the previous bundle"
        );
    }
    // Either way the bundle must still open.
    assert!(Bundle::open(&archive).is_ok());
}

#[test]
fn options_and_the_positional_form_agree() {
    let (temp, root) = tree();
    let positional = temp.path().join("a.pfac");
    let structured = temp.path().join("b.pfac");

    create_bundle(&root, &positional, true, Some("pw")).unwrap();
    create_bundle_with(
        &root,
        &structured,
        &BundleOptions::compressed().with_password("pw"),
    )
    .unwrap();

    let mut one = Bundle::open_with_password(&positional, "pw").unwrap();
    let mut two = Bundle::open_with_password(&structured, "pw").unwrap();

    assert_eq!(
        one.read("art/cover.png").unwrap(),
        two.read("art/cover.png").unwrap()
    );
}
