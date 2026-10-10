//! Writes an export straight into a bundle entry, then streams it back out.
//!
//! ```sh
//! make example-export                       # -> target/example/export.pfac
//! make example-export OUT=target/export.zip
//! ```
//!
//! Exporters that take a path (`to_flac(input, "out.flac")`) have a twin that
//! takes a writer (`to_flac_into(input, writer)`). A bundle entry is one: ask
//! the [`BundleWriter`] for an entry, hand `&mut entry` to the exporter, and
//! `finish` it. With atome that is:
//!
//! ```ignore
//! let mut bundle = BundleWriter::create("project.pfac", &BundleOptions::stored())?;
//!
//! let mut entry = bundle.entry("music/take.flac")?;
//! let summary = atome::export::to_flac_into("take.mp3", &mut entry)?;
//! entry.finish()?;
//!
//! bundle.finish()?;
//! ```
//!
//! vtome's `transcode_into(input, &mut entry, ...)` is the same call for video.
//!
//! This example does not depend on either of them. `write_tone` below is an
//! exporter of the same shape — it writes a header it cannot know yet, goes
//! on, then seeks back to fill it in, which is what a FLAC or an MP4 writer
//! does, and why the entry is [`Seek`] and not just [`Write`]. A zip entry
//! alone could not do that: it is written once, forward.

use std::env;
use std::error::Error;
use std::f32::consts::TAU;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use pfac::{Bundle, BundleOptions, BundleWriter, with_default_extension};

const RATE: u32 = 44_100;

/// A mono 16-bit WAV of `seconds` of `hertz`, into anything that can seek.
///
/// The RIFF and `data` sizes are placeholders until the samples have been
/// written, then patched in. Returns the frames written.
fn write_tone(out: &mut (impl Write + Seek), hertz: f32, seconds: u32) -> io::Result<u32> {
    let start = out.stream_position()?;

    let header = |data_bytes: u32| {
        let mut bytes = Vec::with_capacity(44);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1_u16.to_le_bytes()); // mono
        bytes.extend_from_slice(&RATE.to_le_bytes());
        bytes.extend_from_slice(&(RATE * 2).to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_bytes.to_le_bytes());
        bytes
    };

    out.write_all(&header(0))?;

    let frames = RATE * seconds;
    for frame in 0..frames {
        let sample = ((frame as f32 * hertz * TAU / RATE as f32).sin() * 12_000.0) as i16;
        out.write_all(&sample.to_le_bytes())?;
    }

    let end = out.stream_position()?;
    out.seek(SeekFrom::Start(start))?;
    out.write_all(&header(frames * 2))?;
    out.seek(SeekFrom::Start(end))?;

    Ok(frames)
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = with_default_extension(
        env::args()
            .nth(1)
            .filter(|argument| !argument.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/example/export")
            }),
    );

    // --- write ------------------------------------------------------------

    // Stored, not deflated: only a stored entry can be streamed back out.
    let mut bundle = BundleWriter::create(&output, &BundleOptions::stored())?;

    // Several exports into one bundle. Each entry is its own scratch file,
    // copied in by `finish`; one dropped without it would be left out.
    for (name, hertz) in [("audio/a4.wav", 440.0), ("audio/e5.wav", 659.25)] {
        let mut entry = bundle.entry(name)?;
        let frames = write_tone(&mut entry, hertz, 2)?;
        let bytes = entry.finish()?;

        println!("wrote {name}: {frames} frames, {bytes} bytes");
    }

    bundle.finish()?;
    println!("     -> {}", output.display());

    // --- read -------------------------------------------------------------

    // Straight out of the zip, as `Read + Seek`: the first thing a WAV or FLAC
    // decoder does with it.
    let mut bundle = Bundle::open(&output)?;
    println!("\nbundle holds: {:?}", bundle.names().collect::<Vec<_>>());

    let mut stream = bundle.stream("audio/e5.wav")?;

    let mut header = [0_u8; 44];
    stream.read_exact(&mut header)?;
    let data_bytes = u32::from_le_bytes(header[40..44].try_into()?);
    println!(
        "audio/e5.wav: {} bytes, header says {data_bytes} bytes of samples",
        stream.len()
    );

    // The patched header is what landed, not the placeholder.
    assert_eq!(&header[..4], b"RIFF");
    assert_eq!(u64::from(data_bytes) + 44, stream.len());

    // And it seeks: one second in, 16 samples.
    stream.seek(SeekFrom::Start(44 + u64::from(RATE) * 2))?;
    let mut samples = [0_u8; 32];
    stream.read_exact(&mut samples)?;
    println!("one second in: {:?} ...", &samples[..8]);

    Ok(())
}
