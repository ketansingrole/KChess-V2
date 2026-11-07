use super::{
    BRIDGE_FOLDER, INSTALLING, Install, InstallLock, Package, Release, Signer, Workspace, aside,
    asset_name, detect, digest_for, download_signed, downloads_in, incoming, last_install_error,
    latest, offer, parse_version, rename_bridge_folder, replace_by, signed_alike, swap_file, tidy,
};
use sha2::{Digest, Sha256};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

/// "Could not download the checksums" failed one full run in forty with nothing logged; a
/// failure's log line now carries the error and each cause under it.
#[test]
fn a_failed_request_logs_every_cause() {
    #[derive(Debug)]
    struct Request(std::io::Error);
    impl std::fmt::Display for Request {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "error sending request")
        }
    }
    impl std::error::Error for Request {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }
    let reset = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "connection reset");
    assert_eq!(
        super::causes(&Request(reset)),
        "error sending request: connection reset"
    );
}

/// Held by tests that start processes and by those that check a lock is free once dropped: a
/// process started meanwhile can briefly share another test's lock.
static PROCESSES: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn processes() -> std::sync::MutexGuard<'static, ()> {
    PROCESSES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Wait until the installation's lock is free once its holder has let it go. A process another
/// test forks meanwhile (the end-to-end tests' engines, which do not hold `PROCESSES`) holds a
/// copy of the lock until its own exec closes it, so the lock is given a moment.
fn wait_unlocked(root: &Path, installed: &Path) {
    let started = std::time::Instant::now();
    while InstallLock::take(root, installed).is_err() {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "the installation stays locked after its holder let it go"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("kchess-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

#[test]
fn the_bridge_folder_becomes_kchess_app() {
    let folder = scratch("bridge-folder");
    let bridge = folder.join(BRIDGE_FOLDER);
    std::fs::create_dir_all(bridge.join("Contents")).unwrap();
    assert_eq!(
        rename_bridge_folder(&bridge).unwrap(),
        Some(folder.join("KChess.app"))
    );
    assert!(folder.join("KChess.app/Contents").is_dir() && !bridge.exists());
    assert_eq!(
        rename_bridge_folder(&folder.join("KChess.app")).unwrap(),
        None
    );

    std::fs::create_dir_all(&bridge).unwrap();
    assert_eq!(
        rename_bridge_folder(&bridge).unwrap(),
        None,
        "never over KChess.app"
    );
    assert!(bridge.is_dir());
    let _ = std::fs::remove_dir_all(folder);
}

#[test]
fn only_a_package_updates_itself() {
    let bundle = Path::new("/Applications/KChess.app");
    assert_eq!(
        detect("macos", &bundle.join("Contents/MacOS/kchess"), None),
        Some(Install {
            package: Package::MacZip,
            path: bundle.to_path_buf()
        })
    );
    assert_eq!(
        detect("macos", Path::new("/repo/target/release/KChess"), None),
        None
    );

    let folder = scratch("detect");
    let installed = folder.join("Programs/kchess");
    std::fs::create_dir_all(&installed).unwrap();
    let exe = installed.join("KChess.exe");
    assert_eq!(
        detect("windows", &exe, None),
        None,
        "no uninstaller beside it"
    );
    std::fs::write(installed.join("Uninstall KChess.exe"), "").unwrap();
    assert_eq!(
        detect("windows", &exe, None),
        Some(Install {
            package: Package::WindowsInstaller,
            path: installed.clone()
        })
    );

    let appimage = folder.join("KChess.AppImage");
    std::fs::write(&appimage, "").unwrap();
    let mounted = Path::new("/tmp/.mount_KChess/kchess");
    assert_eq!(
        detect("linux", mounted, Some(appimage.clone())),
        Some(Install {
            package: Package::AppImage,
            path: appimage
        })
    );
    assert_eq!(
        detect("linux", Path::new("/opt/KChess/kchess"), None),
        Some(Install {
            package: Package::Deb,
            path: PathBuf::from("/opt/KChess/kchess")
        })
    );
    assert_eq!(
        detect("linux", Path::new("/home/me/kchess/kchess"), None),
        None
    );
    let _ = std::fs::remove_dir_all(&folder);
}

/// Only macOS exchanges two paths in one step; elsewhere a bundle is never replaced.
#[cfg(target_os = "macos")]
#[test]
fn the_new_bundle_takes_the_old_ones_place() {
    let folder = scratch("replace");
    let (installed, staged) = (folder.join("KChess.app"), folder.join("new/KChess.app"));
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::create_dir_all(&staged).unwrap();
    std::fs::write(installed.join("kchess"), "old").unwrap();
    std::fs::write(staged.join("kchess"), "new").unwrap();
    super::replace(&staged, &installed).unwrap();
    assert_eq!(
        std::fs::read_to_string(installed.join("kchess")).unwrap(),
        "new"
    );
    assert!(!folder.join(".KChess.app-old").exists());
    assert!(!incoming(&installed).exists());
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn an_interrupted_replacement_leaves_the_installed_app_whole() {
    let folder = scratch("interrupted");
    let installed = folder.join("KChess.app");
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::write(installed.join("kchess"), "old").unwrap();
    // Stopped halfway through copying the new bundle beside the installed one.
    let partial = incoming(&installed);
    std::fs::create_dir_all(&partial).unwrap();
    std::fs::write(partial.join("half"), "").unwrap();
    let install = Install {
        package: Package::MacZip,
        path: installed.clone(),
    };
    tidy(Some(&install), None);
    assert!(!partial.exists());
    assert_eq!(
        std::fs::read_to_string(installed.join("kchess")).unwrap(),
        "old"
    );
    let _ = std::fs::remove_dir_all(&folder);
}

/// Where the disk cannot exchange the two bundles, nothing moves the installed app: a crash
/// between moving it aside and moving the new one in would leave no app to open.
#[test]
fn without_an_exchange_the_installed_app_stays_in_place() {
    let folder = scratch("no-exchange");
    let (installed, staged) = (folder.join("KChess.app"), folder.join("new/KChess.app"));
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::create_dir_all(&staged).unwrap();
    std::fs::write(installed.join("kchess"), "old").unwrap();
    std::fs::write(staged.join("kchess"), "new").unwrap();
    let unsupported = |_: &Path, _: &Path| Err(std::io::ErrorKind::Unsupported.into());
    let failed = replace_by(&staged, &installed, unsupported).unwrap_err();
    assert!(failed.contains("Install the update by hand"), "{failed}");
    assert_eq!(
        std::fs::read_to_string(installed.join("kchess")).unwrap(),
        "old"
    );
    assert!(!aside(&installed).exists());
    assert!(!incoming(&installed).exists());
    let _ = std::fs::remove_dir_all(&folder);
}

/// Two KChess processes from one installation: only one installs at a time, and a start
/// meanwhile leaves the installer's new copy alone.
#[test]
fn one_installer_at_a_time_for_each_installation() {
    let _processes = processes();
    let folder = scratch("install-lock");
    let root = downloads_in(&folder.join("cache")).unwrap();
    let installed = folder.join("KChess.AppImage");
    std::fs::write(&installed, "old").unwrap();
    let install = Install {
        package: Package::AppImage,
        path: installed.clone(),
    };
    let first = InstallLock::take(&root, &installed).unwrap();
    let second = InstallLock::take(&root, &installed).err().unwrap();
    assert!(second.contains("Another KChess is installing"), "{second}");
    // Paused after putting the new copy beside the installed one.
    std::fs::write(incoming(&installed), "new").unwrap();
    tidy(Some(&install), Some(&root));
    assert!(incoming(&installed).exists(), "the installer's copy stays");
    // An app starting clears stopped apps' folders, never an installer's files.
    let package = InstallLock::path(&root, &installed).with_extension("deb");
    std::fs::write(&package, "deb").unwrap();
    let _workspace = Workspace::open(&root).unwrap();
    assert!(InstallLock::path(&root, &installed).exists());
    assert!(package.exists());
    drop(first);
    wait_unlocked(&root, &installed);
    tidy(Some(&install), Some(&root));
    assert!(
        !incoming(&installed).exists(),
        "an interrupted install is tidied"
    );
    assert!(InstallLock::take(&root, &installed).is_ok());
    let _ = std::fs::remove_dir_all(&folder);
}

/// A .deb whose helper stopped before installing it goes on the next start, once no helper
/// holds the installation; one that failed is reported once.
#[test]
fn a_deb_helpers_package_and_failure_outlive_the_app() {
    let _processes = processes();
    let folder = scratch("deb-helper");
    let root = downloads_in(&folder.join("cache")).unwrap();
    let install = Install {
        package: Package::Deb,
        path: folder.join("kchess"),
    };
    let held = InstallLock::path(&root, &install.path);
    assert!(
        held.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(INSTALLING)
    );
    let package = held.with_extension("deb");
    std::fs::write(&package, "deb").unwrap();
    // The helper still waits for a password: its package stays.
    let helper = InstallLock::take(&root, &install.path).unwrap();
    tidy(Some(&install), Some(&root));
    assert!(package.exists());
    drop(helper);
    wait_unlocked(&root, &install.path);
    tidy(Some(&install), Some(&root));
    assert!(!package.exists());
    // An earlier version's helper notes only that it failed.
    std::fs::write(held.with_extension("failed"), "failed").unwrap();
    assert!(last_install_error(Some(&install), Some(&root)).is_some());
    assert_eq!(last_install_error(Some(&install), Some(&root)), None);
    // Installed, or the password prompt dismissed: nothing to show. A failure is, once.
    let outcome = held.with_extension("outcome");
    for quiet in ["0\n", "126\n"] {
        std::fs::write(&outcome, quiet).unwrap();
        assert_eq!(last_install_error(Some(&install), Some(&root)), None);
        assert!(!outcome.exists());
    }
    std::fs::write(&outcome, "1\ndpkg: error processing archive\n").unwrap();
    assert!(last_install_error(Some(&install), Some(&root)).is_some());
    assert_eq!(last_install_error(Some(&install), Some(&root)), None);
    let _ = std::fs::remove_dir_all(&folder);
}

/// Review 2026-10-27 #7: install-on-quit's helper waited for the password while the app's
/// download folder, and the .deb in it, were the exited app's, so a KChess started meanwhile
/// deleted the package before dpkg opened it, and the failure went unseen. The helper now
/// owns the package and the installation's lock until the package manager is done, and the
/// next start reads what happened.
#[cfg(unix)]
#[test]
fn a_start_while_the_deb_helper_waits_for_a_password_leaves_its_package() {
    use std::os::unix::fs::PermissionsExt as _;
    let _processes = processes();
    let folder = scratch("deb-relaunch");
    let root = downloads_in(&folder.join("cache")).unwrap();
    let install = Install {
        package: Package::Deb,
        path: folder.join("kchess"),
    };
    let app = Workspace::open(&root).unwrap();
    let staged = app.path.join("KChess.deb");
    std::fs::write(&staged, "deb").unwrap();
    // Stands in for `pkexec dpkg -i PACKAGE`: waits (for a password) until answered, reads
    // the package as dpkg would, then fails as dpkg does.
    let (asked, answer, read) = (
        folder.join("asked"),
        folder.join("answer"),
        folder.join("read"),
    );
    let elevate = folder.join("pkexec");
    std::fs::write(
        &elevate,
        format!(
            "#!/bin/sh\n: > '{asked}'\nwhile [ ! -e '{answer}' ]; do sleep 0.02; done\n\
             cat \"$3\" > '{read}' || exit 9\necho 'dpkg: error processing archive' >&2\n\
             exit 1\n",
            asked = asked.display(),
            answer = answer.display(),
            read = read.display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&elevate, std::fs::Permissions::from_mode(0o755)).unwrap();
    // The app quitting: it hands the package and the lock over, then exits.
    let quit = folder.join("quit");
    let mut running = std::process::Command::new("sh")
        .arg("-c")
        .arg(r#"while [ ! -e "$1" ]; do sleep 0.02; done"#)
        .arg("app")
        .arg(&quit)
        .spawn()
        .unwrap();
    let lock = InstallLock::take(&root, &install.path).unwrap();
    let mut helper = super::deb_helper(
        &root,
        &staged,
        &install,
        &lock,
        running.id(),
        elevate.as_os_str(),
    )
    .unwrap();
    let package = InstallLock::path(&root, &install.path).with_extension("deb");
    assert!(!staged.exists() && package.exists(), "the helper's package");
    drop(lock);
    drop(app);
    std::fs::write(&quit, "").unwrap();
    running.wait().unwrap();
    let started = std::time::Instant::now();
    while !asked.exists() {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "the helper never asked for permission"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // A KChess starts while the password is asked for: it tidies and clears stopped apps'
    // folders, but the helper's package and lock stay, and there is nothing to report yet.
    tidy(Some(&install), Some(&root));
    let relaunched = Workspace::open(&root).unwrap();
    assert_eq!(last_install_error(Some(&install), Some(&root)), None);
    assert!(
        package.exists(),
        "the package waits for the package manager"
    );
    assert!(InstallLock::take(&root, &install.path).is_err());
    std::fs::write(&answer, "").unwrap();
    assert!(helper.wait().unwrap().success());
    assert_eq!(std::fs::read_to_string(&read).unwrap(), "deb");
    assert!(!package.exists(), "the helper removes its package");
    // The next start says the update was not installed, once.
    drop(relaunched);
    tidy(Some(&install), Some(&root));
    let _next = Workspace::open(&root).unwrap();
    assert_eq!(
        last_install_error(Some(&install), Some(&root)).as_deref(),
        Some("The last update was not installed")
    );
    assert_eq!(last_install_error(Some(&install), Some(&root)), None);
    let _ = std::fs::remove_dir_all(&folder);
}

/// The process finishing an installation after the app exits holds it from the moment it
/// starts: a KChess starting meanwhile neither installs into it nor tidies its package.
#[cfg(unix)]
#[test]
fn an_installer_started_on_quit_keeps_the_installation_until_it_exits() {
    let _processes = processes();
    let folder = scratch("install-handover");
    let root = downloads_in(&folder.join("cache")).unwrap();
    let install = Install {
        package: Package::Deb,
        path: folder.join("kchess"),
    };
    let package = InstallLock::path(&root, &install.path).with_extension("deb");
    std::fs::write(&package, "deb").unwrap();
    let release = folder.join("release");
    let lock = InstallLock::take(&root, &install.path).unwrap();
    // An installer still waiting (for a password, or to finish): moves the lock off its
    // standard input as the helper does, and runs until told to stop.
    let mut installer = std::process::Command::new("sh")
        .arg("-c")
        .arg(r#"exec 9<&0 0</dev/null; while [ ! -e "$1" ]; do sleep 0.02; done"#)
        .arg("installer")
        .arg(&release)
        .stdin(lock.inherited().unwrap())
        .spawn()
        .unwrap();
    // The app has exited.
    drop(lock);
    assert!(InstallLock::take(&root, &install.path).is_err());
    tidy(Some(&install), Some(&root));
    assert!(package.exists(), "the installer's package stays");
    std::fs::write(&release, "").unwrap();
    assert!(installer.wait().unwrap().success());
    // Free once the installer has gone: under the load of three test runs at once a forked
    // copy of the lock outlasted the installer.
    wait_unlocked(&root, &install.path);
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn each_running_app_keeps_its_own_downloads() {
    let _processes = processes();
    let folder = scratch("workspaces");
    let root = downloads_in(&folder).unwrap();
    let first = Workspace::open(&root).unwrap();
    let staged = first.path.join("2026.11.0/update");
    std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
    std::fs::write(&staged, "checked").unwrap();
    // Another app starting (a development build, another profile) leaves it alone.
    let second = Workspace::open(&root).unwrap();
    assert!(staged.exists());
    // Once the first app has stopped, the next one to start clears its folder.
    let stopped = first.path.clone();
    drop(first);
    let _third = Workspace::open(&root).unwrap();
    assert!(!stopped.exists());
    assert!(second.path.exists());
    let _ = std::fs::remove_dir_all(&folder);
}

#[cfg(unix)]
#[test]
fn downloads_never_go_through_a_folder_that_is_not_private() {
    use std::os::unix::fs::PermissionsExt as _;
    let folder = scratch("private");
    let elsewhere = folder.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let cache = folder.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::os::unix::fs::symlink(&elsewhere, cache.join("kchess-update")).unwrap();
    assert!(downloads_in(&cache).is_err(), "a link is refused");
    let fresh = folder.join("fresh");
    let root = downloads_in(&fresh).unwrap();
    for dir in [root.parent().unwrap(), root.as_path()] {
        let mode = std::fs::metadata(dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "only its owner can use {}", dir.display());
    }
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn the_new_appimage_replaces_the_running_one() {
    let folder = scratch("appimage");
    let (installed, staged) = (folder.join("KChess.AppImage"), folder.join("download"));
    std::fs::write(&installed, "old").unwrap();
    std::fs::write(&staged, "new").unwrap();
    swap_file(&staged, &installed).unwrap();
    assert_eq!(std::fs::read_to_string(&installed).unwrap(), "new");
    assert!(!folder.join(".KChess.AppImage.new").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&installed).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "executable");
    }
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn an_update_is_signed_as_the_app_it_replaces() {
    let developer = Signer {
        valid: true,
        identity: Some("TEAM123".into()),
        app: Some("com.kchess.app".into()),
    };
    assert_eq!(signed_alike(&developer, &developer.clone()), Ok(()));
    let other_team = Signer {
        identity: Some("OTHER".into()),
        ..developer.clone()
    };
    assert!(signed_alike(&developer, &other_team).is_err());
    let ad_hoc = Signer {
        valid: true,
        identity: None,
        app: Some("com.kchess.app".into()),
    };
    assert!(
        signed_alike(&developer, &ad_hoc).is_err(),
        "lost its signer"
    );
    // An ad hoc build accepts another ad hoc build of the same app, never a broken one.
    assert_eq!(signed_alike(&ad_hoc, &ad_hoc.clone()), Ok(()));
    let broken = Signer {
        valid: false,
        ..ad_hoc.clone()
    };
    assert!(signed_alike(&ad_hoc, &broken).is_err());
    let other_app = Signer {
        app: Some("com.example.other".into()),
        ..ad_hoc.clone()
    };
    assert!(signed_alike(&ad_hoc, &other_app).is_err());
    // The way off ad hoc signing: a Developer ID build of the same app.
    let ad_hoc_build = Signer {
        app: developer.app.clone(),
        ..ad_hoc.clone()
    };
    assert_eq!(signed_alike(&ad_hoc_build, &developer), Ok(()));
    // Unsigned (Linux, or Windows without a certificate): the checksum is the check.
    assert_eq!(signed_alike(&Signer::default(), &Signer::default()), Ok(()));
}

#[cfg(target_os = "macos")]
#[test]
fn codesign_reads_who_signed_a_bundle() {
    let _processes = processes();
    use super::signer;
    let folder = scratch("codesign");
    let bundle = |id: &str| {
        let app = folder.join(format!("{id}.app"));
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::copy("/usr/bin/true", app.join("Contents/MacOS/kchess")).unwrap();
        std::fs::write(
            app.join("Contents/Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict>\
                 <key>CFBundleIdentifier</key><string>{id}</string>\
                 <key>CFBundleExecutable</key><string>kchess</string></dict></plist>"
            ),
        )
        .unwrap();
        let signed = std::process::Command::new("codesign")
            .args(["--force", "--sign", "-"])
            .arg(&app)
            .output()
            .unwrap();
        assert!(signed.status.success());
        app
    };
    let running = signer(Package::MacZip, &bundle("com.kchess.app"));
    assert!(running.valid);
    assert_eq!(running.identity, None, "ad hoc");
    assert_eq!(running.app.as_deref(), Some("com.kchess.app"));
    let other = signer(Package::MacZip, &bundle("com.example.other"));
    assert!(signed_alike(&running, &other).is_err());
    // A file changed after signing breaks the seal.
    let tampered = bundle("com.kchess.app.tampered");
    std::fs::write(tampered.join("Contents/MacOS/kchess"), "changed").unwrap();
    assert!(!signer(Package::MacZip, &tampered).valid);
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn release_versions_compare_as_numbers() {
    assert_eq!(parse_version("v2026.10.12"), Some([2026, 10, 12]));
    assert!(parse_version("v2026.10.12") > parse_version("2026.9.30"));
    assert!(parse_version("v2026.11.0") > parse_version("2026.10.12"));
    assert_eq!(parse_version("v1.2"), None);
    assert_eq!(parse_version("2026.10.1-beta"), None);
}

#[test]
fn packages_are_named_as_the_release_names_them() {
    let x64 = std::env::consts::ARCH == "x86_64";
    let name = |package| asset_name("2026.11.0", package).unwrap();
    assert!(name(Package::MacZip).starts_with("KChess-2026.11.0-mac-"));
    if x64 {
        assert_eq!(
            name(Package::WindowsInstaller),
            "KChess-2026.11.0-win-x64.exe"
        );
        assert_eq!(
            name(Package::AppImage),
            "KChess-2026.11.0-linux-x86_64.AppImage"
        );
        assert_eq!(name(Package::Deb), "KChess-2026.11.0-linux-amd64.deb");
    }
    let sums = format!("ABC123  {}\nfff  other.zip\n", name(Package::Deb));
    assert_eq!(
        digest_for(&sums, &name(Package::Deb)).as_deref(),
        Some("abc123")
    );
    assert_eq!(digest_for(&sums, "missing.zip"), None);
}

/// Serve `files` (path → body) over HTTP on a local port until the test ends; the base URL.
fn serve(files: Vec<(String, Vec<u8>)>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let files = std::sync::Arc::new(files);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let files = std::sync::Arc::clone(&files);
            std::thread::spawn(move || {
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    request.push(byte[0]);
                }
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap_or("");
                let found = files.iter().find(|(file, _)| file == path);
                let (status, body) = match found {
                    Some((_, body)) => ("200 OK", body.clone()),
                    None => ("404 Not Found", Vec::new()),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(&body);
                // The client hangs up first, or the response can turn into a reset
                // (`kchess_core::loopback::close_after_client`).
                let _ = stream.set_read_timeout(Some(kchess_core::loopback::HANG_UP_WAIT));
                let _ = std::io::copy(&mut stream, &mut std::io::sink());
            });
        }
    });
    base
}

/// A feed whose latest release is `version`, with `package` and `sums` (given the package's
/// real digest); the feed's URL.
fn feed(version: &str, package: &[u8], sums: impl Fn(&str, &str) -> String) -> String {
    feed_signed(version, package, sums, |_| None)
}

/// `feed`, with `sign` giving the checksums' `SHA256SUMS.txt.sig` (none when `None`).
fn feed_signed(
    version: &str,
    package: &[u8],
    sums: impl Fn(&str, &str) -> String,
    sign: impl Fn(&[u8]) -> Option<String>,
) -> String {
    let name = asset_name(version, Package::AppImage).unwrap();
    let digest: String = Sha256::digest(package)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let sums = sums(&digest, &name).into_bytes();
    let signature = sign(&sums);
    // The release names its assets' URLs, so they are served first.
    let mut files = vec![
        (format!("/{name}"), package.to_vec()),
        ("/SHA256SUMS.txt".into(), sums),
    ];
    if let Some(signature) = &signature {
        files.push(("/SHA256SUMS.txt.sig".into(), signature.clone().into_bytes()));
    }
    let assets = serve(files);
    let mut listed = vec![
        serde_json::json!({ "name": name, "browser_download_url": format!("{assets}/{name}"), "size": package.len() }),
        serde_json::json!({ "name": "SHA256SUMS.txt", "browser_download_url": format!("{assets}/SHA256SUMS.txt") }),
    ];
    if signature.is_some() {
        listed.push(serde_json::json!({ "name": "SHA256SUMS.txt.sig", "browser_download_url": format!("{assets}/SHA256SUMS.txt.sig") }));
    }
    let release = serde_json::json!({
        "tag_name": format!("v{version}"),
        "html_url": "https://example.invalid/release",
        "assets": listed,
    });
    serve(vec![(
        "/repos/ketansingrole/KChess-V2/releases/latest".into(),
        release.to_string().into_bytes(),
    )])
}

/// A server that accepts each connection, answers with `head` (nothing when empty), and then
/// sends nothing more; its base URL.
fn stall(head: &'static str) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let mut open = Vec::new();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                request.push(byte[0]);
            }
            let _ = stream.write_all(head.as_bytes());
            open.push(stream);
        }
    });
    base
}

#[test]
fn a_stalled_server_ends_the_download() {
    let folder = scratch("stalled");
    let name = asset_name("2026.11.0", Package::AppImage).unwrap();
    let attempt = |sums: String, package: String| {
        let release: Release = serde_json::from_value(serde_json::json!({
            "tag_name": "v2026.11.0",
            "assets": [
                { "name": name, "browser_download_url": package },
                { "name": "SHA256SUMS.txt", "browser_download_url": sums },
            ],
        }))
        .unwrap();
        let asset = release.assets[0].clone();
        let (progress, _heard) = futures::channel::mpsc::unbounded();
        let target = folder.join("download");
        let started = std::time::Instant::now();
        let result = block_on(unsigned_download(release, asset, target, progress));
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        result
    };
    // No answer at all to the checksums.
    let silent = stall("");
    assert_eq!(
        attempt(format!("{silent}/sums"), format!("{silent}/pkg")),
        Err("Could not download the checksums".into())
    );
    // The package stops halfway through its body.
    let sums = serve(vec![(
        "/SHA256SUMS.txt".into(),
        format!("{}  {name}\n", "0".repeat(64)).into_bytes(),
    )]);
    let halfway = stall("HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nhalf");
    assert_eq!(
        attempt(format!("{sums}/SHA256SUMS.txt"), format!("{halfway}/pkg")),
        Err("The download stopped".into())
    );
    let _ = std::fs::remove_dir_all(&folder);
}

/// A server for `body` whose first `drops` answers stop part way through and hang up; with
/// `ranges`, it sends what a `Range: bytes=N-` asks for. Its base URL, and the `Range` each
/// request asked for.
fn flaky(
    body: Vec<u8>,
    drops: usize,
    ranges: bool,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<Option<u64>>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let heard = std::sync::Arc::clone(&asked);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                request.push(byte[0]);
            }
            let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
            let from = request.lines().find_map(|line| {
                line.strip_prefix("range: bytes=")?
                    .trim_end_matches('-')
                    .trim()
                    .parse::<u64>()
                    .ok()
            });
            let count = {
                let mut asked = heard.lock().unwrap();
                asked.push(from);
                asked.len()
            };
            let start = if ranges {
                from.unwrap_or(0) as usize
            } else {
                0
            };
            let rest = &body[start..];
            let head = if ranges && from.is_some() {
                format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{}\r\n",
                    body.len() - 1,
                    body.len()
                )
            } else {
                "HTTP/1.1 200 OK\r\n".to_string()
            };
            let _ = write!(
                stream,
                "{head}Content-Length: {}\r\nConnection: close\r\n\r\n",
                rest.len()
            );
            if count <= drops {
                // A third of what is left, then the connection goes.
                let _ = stream.write_all(&rest[..rest.len() / 3]);
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Both);
                continue;
            }
            let _ = stream.write_all(rest);
            let _ = stream.set_read_timeout(Some(kchess_core::loopback::HANG_UP_WAIT));
            let _ = std::io::copy(&mut stream, &mut std::io::sink());
        }
    });
    (base, asked)
}

/// A download whose connection dropped started over from nothing, so a slow or unsteady link
/// could never finish a 120 MB update; it now carries on from the bytes already received.
#[test]
fn a_dropped_download_resumes_where_it_stopped() {
    let folder = scratch("resume");
    let package: Vec<u8> = (0..300_000u32).map(|n| (n % 251) as u8).collect();
    let name = asset_name("2026.11.0", Package::AppImage).unwrap();
    let digest: String = Sha256::digest(&package)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let sums = serve(vec![(
        "/SHA256SUMS.txt".into(),
        format!("{digest}  {name}\n").into_bytes(),
    )]);
    let attempt = |server: String, target: &str| {
        let release: Release = serde_json::from_value(serde_json::json!({
            "tag_name": "v2026.11.0",
            "assets": [
                { "name": name, "browser_download_url": format!("{server}/pkg"), "size": package.len() },
                { "name": "SHA256SUMS.txt", "browser_download_url": format!("{sums}/SHA256SUMS.txt") },
            ],
        }))
        .unwrap();
        let asset = release.assets[0].clone();
        let (progress, _heard) = futures::channel::mpsc::unbounded();
        block_on(unsigned_download(
            release,
            asset,
            folder.join(target),
            progress,
        ))
    };

    // Two drops: each request asks only for what is missing, and the file is whole.
    let (server, asked) = flaky(package.clone(), 2, true);
    let file = attempt(server, "ranged").unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), package);
    let asked = asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 3);
    assert_eq!(asked[0], None);
    assert!(asked[1].is_some_and(|from| from > 0));
    assert!(asked[2] > asked[1], "each resume starts further on");

    // A server that ignores `Range` sends the whole package again, which replaces the part.
    let (server, _) = flaky(package.clone(), 1, false);
    let file = attempt(server, "whole").unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), package);

    // A link that keeps dropping gives up after its resumes.
    let (server, asked) = flaky(package.clone(), usize::MAX, true);
    assert_eq!(attempt(server, "gone"), Err("The download stopped".into()));
    assert_eq!(asked.lock().unwrap().len(), 1 + super::RESUMES as usize);
    let _ = std::fs::remove_dir_all(&folder);
}

/// Each start cleared the last one's download, so quitting before installing meant fetching
/// the whole package again; a checked download is now used again, and a part carries on.
#[test]
fn a_download_is_kept_from_one_start_to_the_next() {
    let folder = scratch("kept");
    let package: Vec<u8> = (0..300_000u32).map(|n| (n % 241) as u8).collect();
    let name = asset_name("2026.11.0", Package::AppImage).unwrap();
    let digest: String = Sha256::digest(&package)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let sums = serve(vec![(
        "/SHA256SUMS.txt".into(),
        format!("{digest}  {name}\n").into_bytes(),
    )]);
    let attempt = |server: &str| {
        let release: Release = serde_json::from_value(serde_json::json!({
            "tag_name": "v2026.11.0",
            "assets": [
                { "name": name, "browser_download_url": format!("{server}/pkg"), "size": package.len() },
                { "name": "SHA256SUMS.txt", "browser_download_url": format!("{sums}/SHA256SUMS.txt") },
            ],
        }))
        .unwrap();
        let asset = release.assets[0].clone();
        let (progress, heard) = futures::channel::mpsc::unbounded();
        let result = block_on(unsigned_download(release, asset, folder.clone(), progress));
        (
            result,
            futures::executor::block_on_stream(heard).collect::<Vec<_>>(),
        )
    };
    let part = folder.join(format!("{name}.part"));

    // What an earlier start received, its first third, is asked for no more.
    std::fs::write(&part, &package[..100_000]).unwrap();
    let (server, asked) = flaky(package.clone(), 0, true);
    let (file, heard) = attempt(&server);
    assert_eq!(std::fs::read(file.unwrap()).unwrap(), package);
    assert_eq!(*asked.lock().unwrap(), vec![Some(100_000)]);
    assert!(!part.exists());
    assert_eq!(heard.last(), Some(&(300_000, 300_000)));

    // The checked package is used again without fetching it.
    let (server, asked) = flaky(package.clone(), 0, true);
    let (file, heard) = attempt(&server);
    assert_eq!(std::fs::read(file.unwrap()).unwrap(), package);
    assert!(asked.lock().unwrap().is_empty());
    assert_eq!(heard, vec![(300_000, 300_000)]);

    // A kept package or part that is not this one's is fetched again, whole.
    std::fs::write(folder.join(&name), b"something else").unwrap();
    std::fs::write(&part, vec![7u8; 50_000]).unwrap();
    let (server, asked) = flaky(package.clone(), 0, true);
    let (file, _) = attempt(&server);
    assert_eq!(std::fs::read(file.unwrap()).unwrap(), package);
    assert_eq!(*asked.lock().unwrap(), vec![Some(50_000), None]);
    let _ = std::fs::remove_dir_all(&folder);
}

/// A download reports a step at a time, not once for every chunk the server sends.
#[test]
fn progress_is_reported_a_step_at_a_time() {
    let folder = scratch("progress");
    let package = vec![1u8; 5 * super::PROGRESS_STEP as usize + 123];
    let url = feed("2026.11.0", &package, |digest, name| {
        format!("{digest}  {name}\n")
    });
    let heard = block_on(async move {
        let release: Release = latest(url).await.unwrap();
        let asset = release.assets[0].clone();
        let (progress, heard) = futures::channel::mpsc::unbounded();
        unsigned_download(release, asset, folder.join("pkg"), progress)
            .await
            .unwrap();
        let _ = std::fs::remove_dir_all(&folder);
        futures::executor::block_on_stream(heard).collect::<Vec<_>>()
    });
    let total = package.len() as u64;
    assert!(heard.len() <= 6, "{} reports", heard.len());
    assert_eq!(heard.last(), Some(&(total, total)));
}

/// Only the version being downloaded is kept, never one another KChess is downloading.
#[test]
fn old_kept_downloads_are_removed() {
    let _processes = processes();
    let root = scratch("prune");
    let kept = |version: &str| super::kept_folder(&root, version);
    for version in ["2026.10.1", "2026.11.0", "2026.12.0", "junk"] {
        std::fs::create_dir_all(kept(version)).unwrap();
    }
    let busy = std::fs::File::create(kept("2026.10.1").join(".lock")).unwrap();
    busy.lock().unwrap();
    let current = parse_version("2026.11.0");
    super::prune_kept(&root, |version| version.is_none() || version <= current);
    assert!(kept("2026.10.1").exists(), "still being downloaded");
    assert!(!kept("2026.11.0").exists() && !kept("junk").exists());
    assert!(kept("2026.12.0").exists());
    drop(busy);

    // A running app's start never clears the kept downloads.
    super::clear_stopped(&root);
    assert!(kept("2026.12.0").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_partial_answer_says_where_it_starts() {
    use super::content_range;
    assert_eq!(content_range("bytes 100-199/1000"), Some((100, Some(1000))));
    assert_eq!(content_range("bytes 5-9/*"), Some((5, None)));
    assert_eq!(content_range("items 1-2/3"), None);
}

fn block_on<T: Send + 'static>(work: impl std::future::Future<Output = T> + Send + 'static) -> T {
    futures::executor::block_on(super::on_runtime(work)).unwrap()
}

/// A download over a feed whose checksums carry no signature, as these tests serve: what the
/// download does is under test here, not the release key (`an_update_needs_checksums_signed_by_the_release_key`).
async fn unsigned_download(
    release: Release,
    asset: super::Asset,
    folder: PathBuf,
    progress: futures::channel::mpsc::UnboundedSender<(u64, u64)>,
) -> Result<PathBuf, String> {
    download_signed(release, asset, folder, progress, None).await
}

async fn check_and_download(feed: String, folder: PathBuf) -> Result<PathBuf, String> {
    check_and_download_signed(feed, folder, None).await
}

async fn check_and_download_signed(
    feed: String,
    folder: PathBuf,
    key: Option<Vec<u8>>,
) -> Result<PathBuf, String> {
    let release: Release = latest(feed).await?;
    let version =
        offer(&release, "2026.10.12", Some(Package::AppImage)).ok_or("no update offered")?;
    let name = asset_name(&version, Package::AppImage).unwrap();
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .cloned()
        .unwrap();
    let (progress, _heard) = futures::channel::mpsc::unbounded();
    super::download_signed(release, asset, folder, progress, key).await
}

fn key_pair() -> ring::signature::Ed25519KeyPair {
    let random = ring::rand::SystemRandom::new();
    let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&random).unwrap();
    ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
}

fn base64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A release whose packages and checksums were both replaced (anyone who can change release
/// assets can rewrite SHA256SUMS.txt) is refused: the checksums must carry the release
/// key's signature.
#[test]
fn an_update_needs_checksums_signed_by_the_release_key() {
    use ring::signature::KeyPair as _;
    let folder = scratch("signed");
    let package = b"the new KChess".to_vec();
    let release_key = key_pair();
    let public = release_key.public_key().as_ref().to_vec();
    let sums = |digest: &str, name: &str| format!("{digest}  {name}\n");

    let url = feed_signed("2026.11.0", &package, sums, |sums| {
        Some(base64(release_key.sign(sums).as_ref()))
    });
    let file = block_on(check_and_download_signed(
        url,
        folder.join("ok"),
        Some(public.clone()),
    ))
    .unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), package);

    let url = feed("2026.11.0", &package, sums);
    assert_eq!(
        block_on(check_and_download_signed(
            url,
            folder.join("unsigned"),
            Some(public.clone())
        )),
        Err("The release's checksums are not signed".into())
    );

    let other_key = key_pair();
    let url = feed_signed("2026.11.0", &package, sums, |sums| {
        Some(base64(other_key.sign(sums).as_ref()))
    });
    assert_eq!(
        block_on(check_and_download_signed(
            url,
            folder.join("forged"),
            Some(public.clone())
        )),
        Err("The release's checksums are not signed by KChess".into())
    );
    assert!(!folder.join("forged").exists(), "nothing downloaded");

    let url = feed_signed("2026.11.0", &package, sums, |_| Some("not base64!".into()));
    assert_eq!(
        block_on(check_and_download_signed(
            url,
            folder.join("garbled"),
            Some(public)
        )),
        Err("The release's checksum signature could not be read".into())
    );
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn the_release_key_file_holds_one_key_or_none() {
    use super::parse_update_key;
    assert_eq!(parse_update_key("# only comments\n\n"), None);
    let key = base64(&[7u8; 32]);
    assert_eq!(
        parse_update_key(&format!("# comment\n{key}\n")),
        Some(vec![7u8; 32])
    );
    assert_eq!(parse_update_key(&base64(&[7u8; 31])), None, "wrong length");
    // The committed file is either empty of keys or holds a valid one.
    let committed = include_str!("../../update-key.pub");
    let has_key_line = committed
        .lines()
        .any(|line| !line.trim().is_empty() && !line.trim().starts_with('#'));
    assert_eq!(super::update_key().is_some(), has_key_line);
}

#[test]
fn a_developer_id_update_must_be_signed_through_apple() {
    assert_eq!(
        super::developer_requirement("com.kchess.app", "TEAM123"),
        "=anchor apple generic and identifier \"com.kchess.app\" \
         and certificate leaf[subject.OU] = \"TEAM123\""
    );
}

#[test]
fn an_update_is_found_downloaded_and_checked_from_a_feed() {
    let folder = scratch("feed");
    let package = b"the new KChess".to_vec();
    let url = feed("2026.11.0", &package, |digest, name| {
        format!("{digest}  {name}\n")
    });
    let file = block_on(check_and_download(url, folder.join("ok"))).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), package);

    // A package that does not match its checksum is deleted, never staged.
    let url = feed("2026.11.0", &package, |_, name| {
        format!("{}  {name}\n", "0".repeat(64))
    });
    let refused = block_on(check_and_download(url, folder.join("bad")));
    assert_eq!(
        refused,
        Err("The downloaded update did not match its checksum".into())
    );
    let left: Vec<_> = std::fs::read_dir(folder.join("bad"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name != ".lock")
        .collect();
    assert!(left.is_empty(), "nothing of it is kept: {left:?}");

    let url = feed("2026.11.0", &package, |_, _| "abc  other.zip\n".into());
    assert_eq!(
        block_on(check_and_download(url, folder.join("none"))),
        Err("The release lists no checksum for this package".into())
    );

    // The running version, or one without this platform's package, is not offered.
    let url = feed("2026.10.12", &package, |digest, name| {
        format!("{digest}  {name}\n")
    });
    assert_eq!(
        block_on(check_and_download(url, folder.join("same"))),
        Err("no update offered".into())
    );
    let _ = std::fs::remove_dir_all(&folder);
}
