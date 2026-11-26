fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && std::env::var_os("CARGO_FEATURE_SPARKLE_UPDATER").is_some()
    {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    }
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        windows_icon();
    }
}

/// Embeds the app logo so Explorer, Start, search and the taskbar show it.
fn windows_icon() {
    println!("cargo:rerun-if-changed=assets/windows/rustable.rc");
    println!("cargo:rerun-if-changed=assets/windows/rustable.ico");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("rustable.res");
    let Some(rc) = find_rc() else {
        println!("cargo:warning=rc.exe not found; building without the Windows app icon");
        return;
    };
    let status = std::process::Command::new(rc)
        .args(["/nologo", "/fo"])
        .arg(&out)
        .arg("assets/windows/rustable.rc")
        .status();
    if status.is_ok_and(|status| status.success()) {
        // link.exe accepts compiled resources directly.
        println!("cargo:rustc-link-arg-bins={}", out.display());
    } else {
        println!("cargo:warning=rc.exe failed; building without the Windows app icon");
    }
}

/// rc.exe ships with the Windows SDK that MSVC builds already require.
fn find_rc() -> Option<std::path::PathBuf> {
    if std::process::Command::new("rc.exe").arg("/?").output().is_ok() {
        return Some("rc.exe".into());
    }
    let root = std::path::Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut versions: Vec<_> = std::fs::read_dir(root).ok()?.flatten().map(|entry| entry.path()).collect();
    versions.sort();
    versions.into_iter().rev().map(|version| version.join(r"x64\rc.exe")).find(|rc| rc.is_file())
}
