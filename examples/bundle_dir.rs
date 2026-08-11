//! Bundles `tests/data` and reads it back without unpacking any of it.
//!
//! ```sh
//! make example                          # -> target/example/data.pfac
//! make example OUT=target/data.zip      # any extension works
//! make example OUT=mybundle COMPRESS=1  # deflated: read works, stream does not
//! make example PASSWORD=hunter2         # AES-256
//! ```
//!
//! The whole path, end to end: create, list, stream one entry with a seek in the
//! middle, and read another whole. Nothing is extracted at any point.

use std::env;
use std::error::Error;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use pfac::{Bundle, BundleOptions, create_bundle_with, with_default_extension};

fn main() -> Result<(), Box<dyn Error>> {
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    if !input.is_dir() {
        eprintln!("no such directory: {}", input.display());
        return Ok(());
    }

    // The extension is the caller's: `.pfac` is only what you get by not
    // choosing. `with_default_extension` fills it in when the path has none, so
    // `OUT=mybundle` becomes `mybundle.pfac` and `OUT=data.zip` stays as it is.
    let output = with_default_extension(
        env::args()
            .nth(1)
            .filter(|argument| !argument.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/example/data")
            }),
    );

    let compress = flag("COMPRESS");
    let password = env::var("PASSWORD").ok().filter(|p| !p.is_empty());

    let mut options = BundleOptions {
        compress,
        password: password.clone(),
        ..Default::default()
    };
    options.level = 6;

    // --- write ------------------------------------------------------------

    println!("bundling {}", input.display());
    create_bundle_with(&input, &output, &options)?;

    let source_bytes = total_size(&input)?;
    let bundle_bytes = fs::metadata(&output)?.len();

    println!("     -> {}", output.display());
    println!(
        "        {} in, {} out ({})",
        human(source_bytes),
        human(bundle_bytes),
        if compress { "deflated" } else { "stored" }
    );
    if compress && bundle_bytes >= source_bytes {
        println!("        no smaller: this data is already compressed");
    }

    // --- list -------------------------------------------------------------

    let mut bundle = match &password {
        Some(password) => Bundle::open_with_password(&output, password)?,
        None => Bundle::open(&output)?,
    };

    println!("\nentries");
    for info in bundle.entries()? {
        println!(
            "  {:>9}  {:<9} {}",
            human(info.size),
            if info.is_dir {
                "dir"
            } else if info.streamable() {
                "streamable"
            } else if info.encrypted {
                "encrypted"
            } else {
                "deflated"
            },
            info.name
        );
    }

    // --- stream -----------------------------------------------------------

    let flac = bundle
        .names()
        .find(|name| name.ends_with(".flac"))
        .map(str::to_string);

    if let Some(name) = flac {
        println!("\nstreaming {name}");

        match bundle.stream(&name) {
            Ok(mut stream) => {
                let len = stream.len();

                // Straight out of the bundle: no temp file, no full read.
                let mut magic = [0_u8; 4];
                stream.read_exact(&mut magic)?;
                println!("  magic at 0      {:?}", String::from_utf8_lossy(&magic));

                // The thing a deflate stream cannot do.
                stream.seek(SeekFrom::Start(len / 2))?;
                let mut middle = [0_u8; 8];
                stream.read_exact(&mut middle)?;
                println!("  8 bytes at {:<6} {middle:02x?}", len / 2);

                stream.seek(SeekFrom::End(-4))?;
                let mut tail = [0_u8; 4];
                stream.read_exact(&mut tail)?;
                println!("  last 4 bytes    {tail:02x?}");

                // A second stream, live at the same time as the first.
                let others: Vec<String> = bundle
                    .names()
                    .filter(|other| *other != name && other.ends_with(".flac"))
                    .map(str::to_string)
                    .collect();

                if let Some(other) = others.first() {
                    let mut second = bundle.stream(other)?;
                    let mut head = [0_u8; 4];
                    second.read_exact(&mut head)?;

                    // The first is still where we left it.
                    stream.seek(SeekFrom::Start(0))?;
                    let mut again = [0_u8; 4];
                    stream.read_exact(&mut again)?;

                    println!(
                        "  two streams at once: {} and {} both live",
                        short(&name),
                        short(other)
                    );
                }
            }
            Err(error) => {
                // Which is the whole point of the compressed check.
                println!("  cannot stream: {error}");
            }
        }
    }

    // --- read whole -------------------------------------------------------

    // The smallest entry with anything in it — the case where taking it whole is
    // the sensible thing, and the one path that works on deflated and encrypted
    // entries too.
    let smallest = bundle
        .entries()?
        .into_iter()
        .filter(|info| !info.is_dir && info.size > 0)
        .min_by_key(|info| info.size);

    if let Some(info) = smallest {
        let bytes = bundle.read(&info.name)?;
        println!(
            "\nread {} whole: {} in memory",
            info.name,
            human(bytes.len() as u64)
        );
    }

    // --- the bundled database ---------------------------------------------

    let database = bundle
        .names()
        .find(|name| name.ends_with(".db"))
        .map(str::to_string);

    if let Some(db) = database {
        let info = bundle.info(&db)?;
        println!("\ndatabase {db}");

        if info.size == 0 {
            println!("  0 bytes — tests/data/my_database.db is an empty placeholder,");
            println!("  so there is nothing for SQLite to open. See tests/sqlite.rs for");
            println!("  the same path exercised against a real database.");
        } else if info.streamable() {
            println!("  {} bytes, streamable", info.size);
            println!(
                "  rusqlite: deserialize_read_exact(MAIN_DB, stream, {}, true)",
                info.size
            );
        } else {
            println!(
                "  {} bytes, not streamable — read it whole instead",
                info.size
            );
        }
    }

    println!("\nnothing was extracted; {} is unchanged", output.display());

    Ok(())
}

/// An env var set to anything but `0`, `false`, or empty.
fn flag(name: &str) -> bool {
    env::var(name)
        .map(|value| !matches!(value.as_str(), "" | "0" | "false" | "no"))
        .unwrap_or(false)
}

fn total_size(root: &Path) -> Result<u64, Box<dyn Error>> {
    let mut total = 0;
    for entry in walkdir::WalkDir::new(root) {
        let entry = entry?;
        if entry.file_type().is_file() {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];

    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

/// Just the file name, for a line that would otherwise be mostly path.
fn short(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}
