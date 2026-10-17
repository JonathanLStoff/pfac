//! Shows what registering an icon for an extension would do — and only does it
//! if told to.
//!
//! ```sh
//! make icon                          # print the plan, change nothing
//! make icon APPLY=1                  # actually register it
//! make icon EXT=myapp ICON=a.ico     # a different extension and icon
//! make icon APP_ID=com.you.myapp     # a different ProgID / MIME package name
//! make icon OPEN_WITH=/path/to/app   # also open the extension with a program
//! ```
//!
//! The icon defaults to whichever of `tests/data/pfac.{ico,icns,png}` this
//! desktop can draw, chosen by `pfac::icon_in` rather than by a `cfg!` here.
//!
//! Registering writes to the registry on Windows and to the MIME database on
//! Linux, so this prints the plan and stops unless `APPLY=1`. Everything it
//! would write is per-user; none of it needs administrator rights.

use std::env;
use std::error::Error;
use std::path::PathBuf;

use pfac::icon::{self, FileTypeIcon};

/// The fixtures: one icon per desktop, side by side.
const FIXTURES: &str = "tests/data";

/// Only has to be unique — it is what Windows files the ProgID under and what
/// names the MIME package on Linux. Override it with `APP_ID=`.
const DEFAULT_APP_ID: &str = "com.example.pfac.bundle";

fn main() -> Result<(), Box<dyn Error>> {
    let extension = env::var("EXT").unwrap_or_else(|_| "pfac".to_string());
    let app_id = env::var("APP_ID").unwrap_or_else(|_| DEFAULT_APP_ID.to_string());
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURES);

    println!(
        "this platform reads: {}",
        icon::required_icon_extensions().join(" or ")
    );

    // Each desktop reads a different format, so the default fixture depends on
    // where this is running. `icon_in` is the crate answering that, not this
    // example: the same call is what an installer would make against its own
    // assets directory.
    let icon = match env::var("ICON") {
        Ok(path) => PathBuf::from(path),
        Err(_) => match icon::icon_in(&fixtures) {
            Ok(found) => found,
            Err(error) => {
                eprintln!("{error}");
                eprintln!(
                    "pass one: make icon ICON=/path/to/icon.{}",
                    icon::required_icon_extensions()[0]
                );
                return Ok(());
            }
        },
    };

    if !icon.is_file() {
        eprintln!("no icon at {}", icon.display());
        return Ok(());
    }

    println!("icon:                {}", icon.display());
    println!("app id:              {app_id}");

    let mut spec = FileTypeIcon::new(&extension, &icon, &app_id).description("pfac Bundle");

    // macOS binds document icons to an application bundle rather than to an
    // extension, so there is nothing to register without one.
    if let Ok(bundle) = env::var("APP_BUNDLE") {
        spec = spec.app_bundle(bundle);
    }

    // Without a program, a double-click shows the icon and does nothing more.
    if let Ok(program) = env::var("OPEN_WITH").map(|program| program.trim().to_string()) {
        if !program.is_empty() {
            spec = spec.open_with(program);
        }
    }

    let plan = match icon::plan(&spec) {
        Ok(plan) => plan,
        Err(error) => {
            println!("\ncannot register here:\n  {error}");
            return Ok(());
        }
    };

    println!("\n{plan}");

    if env::var("APPLY").is_ok_and(|value| !matches!(value.as_str(), "" | "0" | "false" | "no")) {
        icon::apply(&plan)?;
        println!("applied. `unregister` undoes it.");
    } else {
        println!("nothing was written. Re-run with APPLY=1 to carry this out.");
    }

    Ok(())
}
