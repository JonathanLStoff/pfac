//! Telling the desktop which icon to show for a file extension.
//!
//! ```no_run
//! use pfac::icon::{FileTypeIcon, icon_in, register};
//!
//! // One icon per desktop sits in the directory; this picks the one this
//! // machine can actually draw.
//! let icon = icon_in("assets/icons")?;
//!
//! let spec = FileTypeIcon::new("pfac", icon, "com.myapp.bundle")
//!     .description("MyApp Bundle");
//!
//! let done = register(&spec)?;
//! println!("{done}");
//! # Ok::<(), pfac::Error>(())
//! ```
//!
//! [`register`] and [`FileTypeIcon`] are re-exported at the crate root as
//! [`pfac::register_icon`](crate::register_icon) and
//! [`pfac::FileTypeIcon`](crate::FileTypeIcon).
//!
//! # This writes to the user's system
//!
//! Registering an association is not a scratch operation: it edits the registry
//! on Windows and the MIME database on Linux. So the work is split in two.
//! [`plan`] works out exactly what would be written and returns it, touching
//! nothing; [`apply`] carries it out. [`register`] is the two together.
//!
//! Print a [`Plan`] before applying it and you get an itemised list of every
//! key, file, and command involved.
//!
//! Everything is written **per user** — `HKEY_CURRENT_USER`, `$XDG_DATA_HOME` —
//! so none of it needs administrator rights and none of it touches other
//! accounts.
//!
//! # The icon format is not the same everywhere
//!
//! `.ico` is a Windows format. The other two desktops will not read one:
//!
//! | Platform | Format | Where the association lives |
//! |---|---|---|
//! | Windows | `.ico` | `HKCU\Software\Classes` |
//! | macOS | `.icns` | an application bundle's `Info.plist` |
//! | Linux | `.png` or `.svg` | freedesktop MIME + icon theme |
//!
//! [`required_icon_extensions`] answers this at runtime, and handing over the
//! wrong format is [`Error::IconFormat`] rather than a silent no-op. Ship all
//! three side by side and let [`icon_in`] choose, rather than writing a `cfg!`
//! of your own.
//!
//! # macOS needs an application bundle
//!
//! Windows and Linux let a bare process claim an extension. macOS does not:
//! Launch Services takes document icons from an application bundle's
//! `Info.plist`, and there is no call that says "this extension gets that icon"
//! on its own. So [`FileTypeIcon::app_bundle`] is required there, and without it
//! [`plan`] fails with [`Error::Unsupported`] explaining why rather than
//! pretending to have done something.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// What the desktop should show for one file extension.
#[derive(Clone, Debug)]
pub struct FileTypeIcon {
    extension: String,
    icon: PathBuf,
    description: String,
    app_id: String,
    mime_type: Option<String>,
    app_bundle: Option<PathBuf>,
}

impl FileTypeIcon {
    /// A new association for `extension`, drawn with `icon`, filed under
    /// `app_id`.
    ///
    /// The extension is taken with or without its leading dot and lowercased —
    /// `"pfac"`, `".pfac"`, and `".PFAC"` all mean the same thing.
    ///
    /// `icon` must be in the format this platform reads; [`icon_in`] picks that
    /// one out of a directory holding all three, and
    /// [`required_icon_extensions`] says which it is.
    ///
    /// `app_id` is the identifier the association is filed under. Windows calls
    /// it a ProgID and shows it to nobody; it only has to be unique, so
    /// reverse-DNS — `"com.myapp.bundle"` — is the convention. On Linux it names
    /// the MIME package file, and on macOS it appears nowhere. It is an argument
    /// rather than something derived from the extension because two applications
    /// claiming `.pfac` must not claim the same key: whichever registered last
    /// would otherwise silently own the other's icon, and `unregister` would
    /// delete an association it did not write.
    pub fn new(
        extension: impl AsRef<str>,
        icon: impl Into<PathBuf>,
        app_id: impl Into<String>,
    ) -> Self {
        let extension = extension
            .as_ref()
            .trim_start_matches('.')
            .to_ascii_lowercase();

        FileTypeIcon {
            description: format!("{} file", extension.to_ascii_uppercase()),
            app_id: app_id.into(),
            extension,
            icon: icon.into(),
            mime_type: None,
            app_bundle: None,
        }
    }

    /// What the desktop calls this kind of file, e.g. `"MyApp Bundle"`.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// The MIME type, on Linux. Defaults to `application/x-<extension>`.
    pub fn mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }

    /// The `.app` bundle the association belongs to. **Required on macOS.**
    ///
    /// Ignored on Windows and Linux, where an association does not need an
    /// application to hang off.
    pub fn app_bundle(mut self, bundle: impl Into<PathBuf>) -> Self {
        self.app_bundle = Some(bundle.into());
        self
    }

    /// The extension, without its dot.
    pub fn extension(&self) -> &str {
        &self.extension
    }

    /// The icon file.
    pub fn icon(&self) -> &Path {
        &self.icon
    }

    /// The identifier the association is filed under, as it was given to
    /// [`new`](FileTypeIcon::new).
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// The MIME type this will register on Linux.
    pub fn resolved_mime_type(&self) -> String {
        self.mime_type
            .clone()
            .unwrap_or_else(|| format!("application/x-{}", self.extension))
    }

    /// The MIME type as the icon theme spells it: `application/x-pfac` becomes
    /// `application-x-pfac`, which is the file name an icon has to carry.
    ///
    /// Only the freedesktop desktops name icons this way; Windows points at the
    /// file directly and macOS names it inside the bundle.
    #[cfg(any(all(unix, not(target_vendor = "apple")), test))]
    fn icon_stem(&self) -> String {
        self.resolved_mime_type().replace('/', "-")
    }
}

/// One thing that has to happen for an association to exist.
///
/// Held rather than done, so a caller can see the whole list first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Set a registry value. Windows.
    RegistryValue {
        /// Key path under `HKEY_CURRENT_USER`.
        key: String,
        /// Value name. Empty means the key's default value.
        name: String,
        /// The string to write.
        value: String,
    },
    /// Write a file, creating the directories above it.
    WriteFile {
        /// Where.
        path: PathBuf,
        /// What.
        contents: String,
    },
    /// Copy the icon into the place the desktop looks for it.
    CopyFile {
        /// The icon as the caller supplied it.
        from: PathBuf,
        /// Where the desktop expects to find it.
        to: PathBuf,
    },
    /// Run a command that refreshes a cache.
    ///
    /// A missing program is not fatal — `gtk-update-icon-cache` is not present
    /// on every desktop, and the association still works without it.
    Run {
        /// The program.
        program: String,
        /// Its arguments.
        args: Vec<String>,
        /// Whether the association is broken if this fails.
        required: bool,
    },
    /// Tell the Windows shell that associations changed, so open windows redraw
    /// without a logout.
    NotifyShell,
}

impl std::fmt::Display for Step {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Step::RegistryValue { key, name, value } => {
                let name = if name.is_empty() { "(default)" } else { name };
                write!(formatter, "registry  HKCU\\{key}  {name} = {value:?}")
            }
            Step::WriteFile { path, .. } => write!(formatter, "write     {}", path.display()),
            Step::CopyFile { from, to } => {
                write!(
                    formatter,
                    "copy      {} -> {}",
                    from.display(),
                    to.display()
                )
            }
            Step::Run { program, args, .. } => {
                write!(formatter, "run       {program} {}", args.join(" "))
            }
            Step::NotifyShell => write!(formatter, "notify    the shell that associations changed"),
        }
    }
}

/// Everything [`apply`] would do, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The platform this was built for.
    pub platform: &'static str,
    /// The steps, in the order they must run.
    pub steps: Vec<Step>,
}

impl std::fmt::Display for Plan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(formatter, "{} — {} steps", self.platform, self.steps.len())?;
        for step in &self.steps {
            writeln!(formatter, "  {step}")?;
        }
        Ok(())
    }
}

/// The icon formats this platform will read, in preference order.
///
/// `["ico"]` on Windows, `["icns"]` on macOS, `["svg", "png"]` on Linux.
pub fn required_icon_extensions() -> &'static [&'static str] {
    if cfg!(windows) {
        &["ico"]
    } else if cfg!(target_vendor = "apple") {
        &["icns"]
    } else {
        &["svg", "png"]
    }
}

/// The icon in `directory` that this desktop can draw.
///
/// An application that ships everywhere ships one icon per desktop side by
/// side — `pfac.ico`, `pfac.icns`, `pfac.png` — and exactly one of them is the
/// right one on any given machine. This picks it in the preference order of
/// [`required_icon_extensions`], so the choice is made at runtime rather than in
/// a `cfg!` at every call site. `tests/data` in this repository is such a
/// directory, and is what `make icon` registers from.
///
/// The first file of a usable format wins, taken in sorted order so the answer
/// does not depend on how the filesystem happens to list the directory.
/// Everything else — audio, databases, `.DS_Store` — is ignored, but a directory
/// holding several images will still hand back whichever sorts first: keep the
/// icons somewhere of their own, or name the file yourself.
///
/// # Errors
///
/// [`Error::Io`] if the directory cannot be read, or holds no icon in a format
/// this platform reads.
///
/// ```no_run
/// let icon = pfac::icon::icon_in("tests/data")?;
///
/// // .ico on Windows, .icns on macOS, .png or .svg elsewhere.
/// println!("{}", icon.display());
/// # Ok::<(), pfac::Error>(())
/// ```
pub fn icon_in(directory: impl AsRef<Path>) -> Result<PathBuf> {
    let directory = directory.as_ref();
    let wanted = required_icon_extensions();

    let mut candidates: Vec<PathBuf> = std::fs::read_dir(directory)
        .map_err(|error| Error::io(directory, error))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();

    candidates.sort();

    // By format first, then by name: a platform that reads two formats has a
    // preference between them, and `svg` before `png` is not alphabetical.
    for format in wanted {
        if let Some(found) = candidates.iter().find(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case(format))
        }) {
            return Ok(found.clone());
        }
    }

    Err(Error::io(
        directory,
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "no {} icon here, which is what this platform draws",
                wanted.join(" or ")
            ),
        ),
    ))
}

/// Whether this platform can register an association at all.
///
/// True everywhere the crate compiles — but on macOS it is only true with
/// [`FileTypeIcon::app_bundle`] set, which this cannot see. Use [`plan`] for a
/// real answer.
pub fn is_supported() -> bool {
    cfg!(any(windows, target_vendor = "apple", target_os = "linux"))
}

/// Works out what registering `spec` would take, without doing any of it.
///
/// Checks the icon exists and is a format this platform reads. Everything after
/// that is arithmetic on paths.
///
/// # Errors
///
/// [`Error::IconFormat`] if the icon is the wrong kind, [`Error::Io`] if it is
/// not there, [`Error::Unsupported`] on a platform — or a configuration — that
/// cannot express the association.
pub fn plan(spec: &FileTypeIcon) -> Result<Plan> {
    if spec.extension.is_empty() {
        return Err(Error::bad_name("", "an extension cannot be empty"));
    }

    if !spec.icon.is_file() {
        return Err(Error::io(
            &spec.icon,
            std::io::Error::new(std::io::ErrorKind::NotFound, "icon file not found"),
        ));
    }

    let wanted = required_icon_extensions();
    let found = spec
        .icon
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if !wanted.contains(&found.as_str()) {
        return Err(Error::IconFormat {
            path: spec.icon.clone(),
            expected: wanted.join(" or "),
        });
    }

    // Relative paths end up written into a registry value or a desktop file,
    // where nothing knows what directory this process happened to be in.
    let icon = spec
        .icon
        .canonicalize()
        .map_err(|error| Error::io(&spec.icon, error))?;

    #[cfg(windows)]
    return Ok(windows_plan(spec, &icon));

    #[cfg(target_vendor = "apple")]
    return apple_plan(spec, &icon);

    #[cfg(all(unix, not(target_vendor = "apple")))]
    return freedesktop_plan(spec, &icon);

    #[cfg(not(any(windows, unix)))]
    {
        let _ = icon;
        Err(Error::Unsupported {
            reason: "no desktop integration is implemented for this platform".to_string(),
        })
    }
}

/// Carries out a [`Plan`].
///
/// Steps run in order and stop at the first required failure. A step marked
/// optional — a cache refresh whose program is not installed — is skipped.
pub fn apply(plan: &Plan) -> Result<()> {
    for step in &plan.steps {
        run_step(step)?;
    }

    Ok(())
}

/// [`plan`] then [`apply`]: registers the association and returns what it did.
pub fn register(spec: &FileTypeIcon) -> Result<Plan> {
    let plan = plan(spec)?;
    apply(&plan)?;

    Ok(plan)
}

/// Removes an association this crate registered.
///
/// Best-effort by design: something already gone is not an error, because the
/// common reason to call this is cleaning up after a half-finished install.
pub fn unregister(spec: &FileTypeIcon) -> Result<()> {
    #[cfg(windows)]
    return windows_unregister(spec);

    #[cfg(target_vendor = "apple")]
    return apple_unregister(spec);

    #[cfg(all(unix, not(target_vendor = "apple")))]
    return freedesktop_unregister(spec);

    #[cfg(not(any(windows, unix)))]
    {
        let _ = spec;
        Err(Error::Unsupported {
            reason: "no desktop integration is implemented for this platform".to_string(),
        })
    }
}

#[cfg(windows)]
fn windows_unregister(spec: &FileTypeIcon) -> Result<()> {
    let root = windows_registry::CURRENT_USER;

    // Both trees, and neither failure matters: a key that is not there is the
    // state this function exists to reach.
    let _ = root.remove_tree(format!("Software\\Classes\\{}", spec.app_id));
    let _ = root.remove_tree(format!("Software\\Classes\\.{}", spec.extension));
    notify_shell();

    Ok(())
}

#[cfg(all(unix, not(target_vendor = "apple")))]
fn freedesktop_unregister(spec: &FileTypeIcon) -> Result<()> {
    let data = xdg_data_home()?;

    let package = data
        .join("mime/packages")
        .join(format!("{}.xml", spec.app_id));
    let _ = std::fs::remove_file(&package);

    // Both candidate locations: which one was used depends on the format of the
    // icon that was registered, which we no longer have.
    for (directory, extension) in icon_targets(&data, spec) {
        let _ = std::fs::remove_file(directory.join(format!("{}.{extension}", spec.icon_stem())));
    }

    let _ = run_step(&Step::Run {
        program: "update-mime-database".to_string(),
        args: vec![data.join("mime").display().to_string()],
        required: false,
    });

    Ok(())
}

/// Nothing to undo that is ours: the association lives in the bundle\'s
/// `Info.plist`, which belongs to the application rather than to this crate.
#[cfg(target_vendor = "apple")]
fn apple_unregister(spec: &FileTypeIcon) -> Result<()> {
    Err(Error::Unsupported {
        reason: format!(
            "macOS associations live in {}\'s Info.plist; \
             remove the CFBundleDocumentTypes entry for .{} and re-run lsregister",
            spec.app_bundle
                .as_deref()
                .unwrap_or(Path::new("the application bundle"))
                .display(),
            spec.extension
        ),
    })
}

// -----------------------------------------------------------------------------
//  Windows
// -----------------------------------------------------------------------------

/// `HKCU\Software\Classes` is the per-user half of `HKEY_CLASSES_ROOT`. Writing
/// there needs no administrator rights and affects only this account.
#[cfg(windows)]
fn windows_plan(spec: &FileTypeIcon, icon: &Path) -> Plan {
    let classes = "Software\\Classes";
    let extension_key = format!("{classes}\\.{}", spec.extension);
    let program_key = format!("{classes}\\{}", spec.app_id);

    Plan {
        platform: "windows",
        steps: vec![
            // The extension points at a ProgID...
            Step::RegistryValue {
                key: extension_key.clone(),
                name: String::new(),
                value: spec.app_id.clone(),
            },
            Step::RegistryValue {
                key: extension_key,
                name: "Content Type".to_string(),
                value: spec.resolved_mime_type(),
            },
            // ...and the ProgID is what carries the name and the icon.
            Step::RegistryValue {
                key: program_key.clone(),
                name: String::new(),
                value: spec.description.clone(),
            },
            Step::RegistryValue {
                key: format!("{program_key}\\DefaultIcon"),
                name: String::new(),
                // `,0` is the icon's index within the file. An .ico usually
                // holds one image, but the suffix is what Explorer expects and
                // leaving it off makes some shells fall back to a blank page.
                value: format!("{},0", icon.display()),
            },
            Step::NotifyShell,
        ],
    }
}

/// Tells Explorer that associations changed. Without it, already-open windows
/// keep the old icon until the next logon.
#[cfg(windows)]
fn notify_shell() {
    use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};

    // No handles, no allocation, nothing to fail: it is a broadcast.
    //
    // The event id is declared `u32` and taken as `i32`, so the cast is the
    // binding's arithmetic rather than ours.
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
}

// -----------------------------------------------------------------------------
//  Linux and the other freedesktop desktops
// -----------------------------------------------------------------------------

/// Where per-user data goes: `$XDG_DATA_HOME`, or `~/.local/share`.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn xdg_data_home() -> Result<PathBuf> {
    if let Some(data) = std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(data));
    }

    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(|home| PathBuf::from(home).join(".local/share"))
        .ok_or_else(|| Error::Unsupported {
            reason: "neither XDG_DATA_HOME nor HOME is set, so there is nowhere to install to"
                .to_string(),
        })
}

/// Where an icon of each format belongs in the hicolor theme.
///
/// SVG goes in `scalable`; a PNG has to claim a pixel size, and 48 is the one
/// every file manager asks for.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn icon_targets(data: &Path, _spec: &FileTypeIcon) -> Vec<(PathBuf, &'static str)> {
    vec![
        (data.join("icons/hicolor/scalable/mimetypes"), "svg"),
        (data.join("icons/hicolor/48x48/mimetypes"), "png"),
    ]
}

#[cfg(all(unix, not(target_vendor = "apple")))]
fn freedesktop_plan(spec: &FileTypeIcon, icon: &Path) -> Result<Plan> {
    let data = xdg_data_home()?;
    let mime = spec.resolved_mime_type();
    let stem = spec.icon_stem();

    let format = icon
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();

    let directory = if format == "svg" {
        data.join("icons/hicolor/scalable/mimetypes")
    } else {
        data.join("icons/hicolor/48x48/mimetypes")
    };

    Ok(Plan {
        platform: "freedesktop",
        steps: vec![
            // The MIME database learns that *.ext is this type...
            Step::WriteFile {
                path: data
                    .join("mime/packages")
                    .join(format!("{}.xml", spec.app_id)),
                contents: mime_package_xml(&mime, &spec.extension, &spec.description),
            },
            // ...and the icon theme learns what that type looks like. The file
            // name is the MIME type with the slash swapped for a dash; nothing
            // else will be looked up.
            Step::CopyFile {
                from: icon.to_path_buf(),
                to: directory.join(format!("{stem}.{format}")),
            },
            Step::Run {
                program: "update-mime-database".to_string(),
                args: vec![data.join("mime").display().to_string()],
                required: true,
            },
            Step::Run {
                program: "gtk-update-icon-cache".to_string(),
                args: vec![
                    "-f".to_string(),
                    "-t".to_string(),
                    data.join("icons/hicolor").display().to_string(),
                ],
                // Not every desktop has GTK, and the association works without
                // the cache — the theme is read from disk when it is missing.
                required: false,
            },
        ],
    })
}

/// The `shared-mime-info` package that teaches the database one glob.
#[cfg(any(all(unix, not(target_vendor = "apple")), test))]
fn mime_package_xml(mime: &str, extension: &str, description: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="{}">
    <comment>{}</comment>
    <glob pattern="*.{}"/>
  </mime-type>
</mime-info>
"#,
        escape_xml(mime),
        escape_xml(description),
        escape_xml(extension)
    )
}

/// The five predefined entities. A description is caller-supplied text going
/// into a document a system tool will parse.
#[cfg(any(all(unix, not(target_vendor = "apple")), test))]
fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// -----------------------------------------------------------------------------
//  macOS
// -----------------------------------------------------------------------------

/// macOS binds document icons to an application, not to a running process.
///
/// Launch Services reads `CFBundleDocumentTypes` out of a bundle's `Info.plist`
/// and takes the icon from that bundle's `Resources`. There is no call that
/// associates an extension with an icon on its own — which is why this needs
/// [`FileTypeIcon::app_bundle`] and fails clearly without it.
#[cfg(target_vendor = "apple")]
fn apple_plan(spec: &FileTypeIcon, icon: &Path) -> Result<Plan> {
    let Some(bundle) = spec.app_bundle.as_ref() else {
        return Err(Error::Unsupported {
            reason: format!(
                "macOS takes document icons from an application bundle's Info.plist, \
                 not from a running process — there is no per-extension icon API to call. \
                 Give FileTypeIcon::app_bundle the .app that owns .{}, \
                 or ship the CFBundleDocumentTypes entry in that bundle at build time",
                spec.extension
            ),
        });
    };

    if !bundle.join("Contents/Info.plist").is_file() {
        return Err(Error::io(
            bundle.join("Contents/Info.plist"),
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "not an application bundle: no Contents/Info.plist",
            ),
        ));
    }

    let plist = bundle.join("Contents/Info.plist");
    let plist_text = plist.display().to_string();
    let icon_name = icon
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("document.icns")
        .to_string();
    let stem = icon_name.trim_end_matches(".icns").to_string();

    // PlistBuddy ships with macOS, so editing the plist needs no dependency and
    // no plist parser of our own. `-c` takes one command; several are chained by
    // running it several times.
    let buddy = |command: String| Step::Run {
        program: "/usr/libexec/PlistBuddy".to_string(),
        args: vec!["-c".to_string(), command, plist_text.clone()],
        // Adding an array that already exists is an error from PlistBuddy and a
        // no-op for us, so none of the plist edits are required to succeed.
        required: false,
    };

    Ok(Plan {
        platform: "macos",
        steps: vec![
            Step::CopyFile {
                from: icon.to_path_buf(),
                to: bundle.join("Contents/Resources").join(&icon_name),
            },
            buddy("Add :CFBundleDocumentTypes array".to_string()),
            buddy("Add :CFBundleDocumentTypes:0 dict".to_string()),
            buddy(format!(
                "Add :CFBundleDocumentTypes:0:CFBundleTypeName string {}",
                spec.description
            )),
            buddy("Add :CFBundleDocumentTypes:0:CFBundleTypeExtensions array".to_string()),
            buddy(format!(
                "Add :CFBundleDocumentTypes:0:CFBundleTypeExtensions:0 string {}",
                spec.extension
            )),
            buddy(format!(
                "Add :CFBundleDocumentTypes:0:CFBundleTypeIconFile string {stem}"
            )),
            buddy("Add :CFBundleDocumentTypes:0:CFBundleTypeRole string Viewer".to_string()),
            // Launch Services caches aggressively; without this the Finder keeps
            // showing the old icon until it notices on its own.
            Step::Run {
                program: "/System/Library/Frameworks/CoreServices.framework/Frameworks/\
                          LaunchServices.framework/Support/lsregister"
                    .to_string(),
                args: vec!["-f".to_string(), bundle.display().to_string()],
                required: true,
            },
        ],
    })
}

// -----------------------------------------------------------------------------
//  Running the steps
// -----------------------------------------------------------------------------

fn run_step(step: &Step) -> Result<()> {
    match step {
        Step::RegistryValue { key, name, value } => write_registry(key, name, value),

        Step::WriteFile { path, contents } => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
            }
            std::fs::write(path, contents).map_err(|error| Error::io(path, error))
        }

        Step::CopyFile { from, to } => {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
            }
            std::fs::copy(from, to)
                .map(|_| ())
                .map_err(|error| Error::io(from, error))
        }

        Step::Run {
            program,
            args,
            required,
        } => {
            let outcome = std::process::Command::new(program).args(args).output();

            match outcome {
                Ok(output) if output.status.success() => Ok(()),
                Ok(output) if *required => Err(Error::Shell {
                    step: format!("{program} {}", args.join(" ")),
                    reason: String::from_utf8_lossy(&output.stderr).trim().to_string(),
                }),
                // An optional refresh that failed, or a program that is not
                // installed. The association is already written either way.
                Ok(_) => Ok(()),
                Err(error) if *required => Err(Error::Shell {
                    step: format!("{program} {}", args.join(" ")),
                    reason: error.to_string(),
                }),
                Err(_) => Ok(()),
            }
        }

        Step::NotifyShell => {
            #[cfg(windows)]
            notify_shell();
            Ok(())
        }
    }
}

#[cfg(windows)]
fn write_registry(key: &str, name: &str, value: &str) -> Result<()> {
    let handle = windows_registry::CURRENT_USER
        .create(key)
        .map_err(|error| Error::Shell {
            step: format!("create HKCU\\{key}"),
            reason: error.to_string(),
        })?;

    handle
        .set_string(name, value)
        .map_err(|error| Error::Shell {
            step: format!("set HKCU\\{key} {name}"),
            reason: error.to_string(),
        })
}

#[cfg(not(windows))]
fn write_registry(key: &str, _name: &str, _value: &str) -> Result<()> {
    Err(Error::Unsupported {
        reason: format!("there is no registry on this platform to write HKCU\\{key} to"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixtures `make icon` registers from: one icon per desktop.
    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
    }

    #[test]
    fn an_extension_is_taken_with_or_without_its_dot() {
        for given in ["pfac", ".pfac", ".PFAC", "PFAC"] {
            let spec = FileTypeIcon::new(given, "icon.ico", "com.myapp.bundle");
            assert_eq!(spec.extension(), "pfac", "{given} was not normalised");
        }
    }

    #[test]
    fn the_app_id_is_the_callers_and_the_rest_is_derived() {
        let spec = FileTypeIcon::new(".pfac", "icon.ico", "com.myapp.bundle");

        assert_eq!(spec.app_id(), "com.myapp.bundle");
        assert_eq!(spec.resolved_mime_type(), "application/x-pfac");
        assert_eq!(spec.description, "PFAC file");
    }

    #[test]
    fn overrides_win() {
        let spec = FileTypeIcon::new("pfac", "icon.ico", "com.myapp.bundle")
            .description("MyApp Bundle")
            .mime_type("application/vnd.myapp.bundle");

        assert_eq!(spec.description, "MyApp Bundle");
        assert_eq!(spec.resolved_mime_type(), "application/vnd.myapp.bundle");
    }

    #[test]
    fn the_icon_stem_is_the_mime_type_with_a_dash() {
        let spec = FileTypeIcon::new("pfac", "icon.png", "com.myapp.bundle")
            .mime_type("application/x-my-bundle");

        assert_eq!(spec.icon_stem(), "application-x-my-bundle");
    }

    #[test]
    fn a_missing_icon_is_reported_before_anything_is_written() {
        let spec = FileTypeIcon::new("pfac", "definitely/not/here.ico", "com.myapp.bundle");

        assert!(matches!(plan(&spec), Err(Error::Io { .. })));
    }

    /// The fixture directory holds `pfac.ico`, `pfac.icns`, and `pfac.png`, so
    /// wherever this runs there is exactly one right answer.
    #[test]
    fn the_fixtures_offer_an_icon_this_platform_can_draw() {
        let icon = icon_in(fixtures()).expect("tests/data holds an icon for every desktop");

        let format = icon
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        assert!(
            required_icon_extensions().contains(&format.as_str()),
            "picked {} on a platform that reads {}",
            icon.display(),
            required_icon_extensions().join(" or ")
        );
        assert!(icon.is_file(), "{} is not there", icon.display());
    }

    /// Which is the whole point: the fixture chosen here is one `plan` accepts,
    /// so `make icon` has something to show on any of the three desktops.
    #[test]
    fn the_fixture_this_platform_picks_is_one_it_will_also_plan_with() {
        let icon = icon_in(fixtures()).unwrap();
        let spec = FileTypeIcon::new("pfac", &icon, "com.example.pfac.bundle");

        match plan(&spec) {
            Ok(_) => {}
            // macOS refuses any plan without an application bundle, which is
            // about the bundle rather than about the icon this picked.
            Err(Error::Unsupported { reason }) if cfg!(target_vendor = "apple") => {
                assert!(reason.contains("app_bundle"), "{reason}");
            }
            Err(error) => panic!("{} was not usable here: {error}", icon.display()),
        }
    }

    #[test]
    fn a_directory_without_a_usable_icon_says_so() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join("notes.txt"), b"no icons here").unwrap();

        let Err(Error::Io { source, .. }) = icon_in(temp.path()) else {
            panic!("a directory with no icon in it cannot produce one");
        };

        assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn the_preferred_format_wins_over_the_merely_acceptable() {
        let temp = tempfile::TempDir::new().unwrap();

        for format in required_icon_extensions() {
            std::fs::write(temp.path().join(format!("app.{format}")), b"icon").unwrap();
        }

        let picked = icon_in(temp.path()).unwrap();

        assert_eq!(
            picked.extension().unwrap(),
            required_icon_extensions()[0],
            "the first listed format is the preferred one"
        );
    }

    #[test]
    fn the_wrong_icon_format_for_this_platform_is_refused() {
        let temp = tempfile::TempDir::new().unwrap();

        // Deliberately the format *no* platform accepts, so this holds wherever
        // the suite runs.
        let wrong = temp.path().join("icon.bmp");
        std::fs::write(&wrong, b"not really an image").unwrap();

        let spec = FileTypeIcon::new("pfac", &wrong, "com.myapp.bundle");
        let Err(Error::IconFormat { expected, .. }) = plan(&spec) else {
            panic!("a .bmp should not be accepted on any platform");
        };

        assert_eq!(expected, required_icon_extensions().join(" or "));
    }

    #[test]
    fn required_formats_match_the_platform() {
        let formats = required_icon_extensions();

        if cfg!(windows) {
            assert_eq!(formats, ["ico"]);
        } else if cfg!(target_vendor = "apple") {
            assert_eq!(formats, ["icns"]);
        } else {
            assert!(formats.contains(&"png"));
            assert!(formats.contains(&"svg"));
        }
    }

    #[test]
    fn the_mime_package_escapes_what_a_caller_supplied() {
        let xml = mime_package_xml("application/x-pfac", "pfac", r#"Bob & Alice's <"bundle">"#);

        assert!(xml.contains("&amp;"), "{xml}");
        assert!(xml.contains("&lt;"), "{xml}");
        assert!(xml.contains("&quot;"), "{xml}");
        assert!(xml.contains("&apos;"), "{xml}");
        assert!(!xml.contains("<\"bundle\">"), "raw markup survived: {xml}");
    }

    #[test]
    fn a_plan_prints_every_step() {
        let step = Step::RegistryValue {
            key: "Software\\Classes\\.pfac".to_string(),
            name: String::new(),
            value: "com.myapp.bundle".to_string(),
        };

        let printed = step.to_string();
        assert!(printed.contains("(default)"), "{printed}");
        assert!(printed.contains("com.myapp.bundle"), "{printed}");
    }

    /// The one that documents the platform difference rather than asserting a
    /// behaviour: on macOS a bare extension cannot be registered at all.
    #[cfg(target_vendor = "apple")]
    #[test]
    fn macos_without_a_bundle_explains_itself() {
        let temp = tempfile::TempDir::new().unwrap();
        let icon = temp.path().join("icon.icns");
        std::fs::write(&icon, b"icns").unwrap();

        let spec = FileTypeIcon::new("pfac", &icon, "com.myapp.bundle");
        let Err(Error::Unsupported { reason }) = plan(&spec) else {
            panic!("macOS cannot register an extension without an app bundle");
        };

        assert!(reason.contains("app_bundle"), "unhelpful reason: {reason}");
    }

    /// And with a bundle it produces a plan — checked without running it, since
    /// applying would edit Launch Services on whatever machine this runs on.
    #[cfg(target_vendor = "apple")]
    #[test]
    fn macos_with_a_bundle_plans_but_is_not_applied_here() {
        let temp = tempfile::TempDir::new().unwrap();
        let icon = temp.path().join("bundle.icns");
        std::fs::write(&icon, b"icns").unwrap();

        let bundle = temp.path().join("MyApp.app");
        std::fs::create_dir_all(bundle.join("Contents")).unwrap();
        std::fs::write(bundle.join("Contents/Info.plist"), b"<plist/>").unwrap();

        let spec = FileTypeIcon::new("pfac", &icon, "com.myapp.bundle")
            .description("MyApp Bundle")
            .app_bundle(&bundle);

        let plan = plan(&spec).unwrap();

        assert_eq!(plan.platform, "macos");
        assert!(matches!(plan.steps.first(), Some(Step::CopyFile { .. })));
        assert!(
            plan.steps.iter().any(|step| matches!(
                step,
                Step::Run { program, .. } if program.ends_with("lsregister")
            )),
            "Launch Services is never told, so the icon would not appear"
        );
    }
}
