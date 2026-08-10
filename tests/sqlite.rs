//! Proof that the stream feeds `rusqlite` with no temp file.
//!
//! The crate docs claim an [`EntryReader`](pfac::EntryReader) can be handed
//! straight to `Connection::deserialize_read_exact`, which wants a `Read` and a
//! length. This is that claim, executed.

use std::fs;

use pfac::{Bundle, create_bundle};
use rusqlite::{Connection, MAIN_DB};
use tempfile::TempDir;

/// Builds a real SQLite file, bundles it uncompressed, and returns the bundle's
/// path along with the temp dir keeping it alive.
fn bundled_database() -> (TempDir, std::path::PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("input");
    fs::create_dir_all(root.join("meta")).unwrap();

    let db_path = root.join("meta").join("project.db");
    let connection = Connection::open(&db_path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE assets (id INTEGER PRIMARY KEY, path TEXT NOT NULL);
             INSERT INTO assets (path) VALUES ('music/take.flac'), ('art/cover.png');",
        )
        .unwrap();
    // Fold the WAL back in, so the file on disk is the whole database.
    connection
        .pragma_update(None, "wal_checkpoint", "TRUNCATE")
        .unwrap();
    drop(connection);

    let archive = temp.path().join("assets.pfac");
    create_bundle(&root, &archive, false, None).unwrap();

    (temp, archive)
}

#[test]
fn a_bundled_database_opens_straight_from_the_stream() {
    let (_temp, archive) = bundled_database();

    let mut bundle = Bundle::open(&archive).unwrap();
    let stream = bundle.stream("meta/project.db").unwrap();
    let size = stream.len() as usize;

    // No temp file, no intermediate Vec: the bytes go from the bundle into
    // SQLite's own buffer.
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .deserialize_read_exact(MAIN_DB, stream, size, true)
        .unwrap();

    let count: i64 = connection
        .query_row("SELECT count(*) FROM assets", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);

    let first: String = connection
        .query_row("SELECT path FROM assets ORDER BY id LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(first, "music/take.flac");
}

#[test]
fn the_read_into_memory_route_works_too() {
    let (_temp, archive) = bundled_database();

    let mut bundle = Bundle::open(&archive).unwrap();
    let bytes = bundle.read("meta/project.db").unwrap();

    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .deserialize_read_exact(MAIN_DB, bytes.as_slice(), bytes.len(), true)
        .unwrap();

    let count: i64 = connection
        .query_row("SELECT count(*) FROM assets", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

/// A compressed bundle has no streamable database, so the memory route is the
/// only way in. Worth asserting, since it is the reason `read` exists.
#[test]
fn a_compressed_bundle_still_yields_a_database_through_read() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("input");
    fs::create_dir_all(&root).unwrap();

    let db_path = root.join("project.db");
    let connection = Connection::open(&db_path).unwrap();
    connection
        .execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (42);")
        .unwrap();
    connection
        .pragma_update(None, "wal_checkpoint", "TRUNCATE")
        .unwrap();
    drop(connection);

    let archive = temp.path().join("packed.pfac");
    create_bundle(&root, &archive, true, None).unwrap();

    let mut bundle = Bundle::open(&archive).unwrap();
    assert!(
        bundle.stream("project.db").is_err(),
        "compressed: no stream"
    );

    let bytes = bundle.read("project.db").unwrap();
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .deserialize_read_exact(MAIN_DB, bytes.as_slice(), bytes.len(), true)
        .unwrap();

    let x: i64 = connection
        .query_row("SELECT x FROM t", [], |row| row.get(0))
        .unwrap();
    assert_eq!(x, 42);
}
