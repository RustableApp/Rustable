//! Cargo-only packaging helper. Never runs the application or accesses databases.
mod updates;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const SQLITE_VERSION: &str = "0.13.3";
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}
fn require(path: &Path, reason: &str) -> Result<()> {
    if !path.is_file() {
        return Err(format!("Missing {}: {reason}", path.display()).into());
    }
    Ok(())
}
fn run(command: &mut Command) -> Result<()> {
    let status = command.status()?;
    if !status.success() {
        return Err(format!("Command failed ({status}): {command:?}").into());
    }
    Ok(())
}
fn version_matches(binary: &Path, version: &str) -> bool {
    Command::new(binary)
        .arg("--version")
        .output()
        .is_ok_and(|out| {
            out.status.success()
                && String::from_utf8_lossy(&out.stdout)
                    .split_whitespace()
                    .any(|word| word == version)
        })
}
fn build_servers(root: &Path) -> Result<()> {
    let sqlite = root.join(format!("target/lsp-tools/bin/syntaqlite{}", env::consts::EXE_SUFFIX));
    if !version_matches(&sqlite, SQLITE_VERSION) {
        run(Command::new("mise").current_dir(root).args([
            "exec",
            "rust@stable",
            "--",
            "cargo",
            "install",
            "syntaqlite-cli",
            "--version",
            SQLITE_VERSION,
            "--locked",
            "--root",
            "target/lsp-tools",
            "--no-default-features",
            "--features",
            "bundled-sqlite-dialect",
        ]))?;
    }
    let pg = root.join(format!("target/pg-language-server/release/rustable-postgres-language-server{}", env::consts::EXE_SUFFIX));
    if !pg.is_file() {
        let tree_sitter = root.join(format!("target/language-server/bin/tree-sitter{}", env::consts::EXE_SUFFIX));
        if !version_matches(&tree_sitter, "0.25.9") {
            run(Command::new("mise").current_dir(root).args([
                "exec",
                "rust@stable",
                "--",
                "cargo",
                "install",
                "tree-sitter-cli",
                "--version",
                "0.25.9",
                "--locked",
                "--root",
                "target/language-server",
            ]))?;
        }
        let mut paths = vec![root.join("target/language-server/bin")];
        paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
        run(Command::new("mise")
            .current_dir(root)
            .env("PATH", env::join_paths(paths)?)
            .args([
                "exec",
                "rust@stable",
                "node@22.23.2",
                "--",
                "cargo",
                "build",
                "--locked",
                "--manifest-path",
                "tools/postgres-language-server/Cargo.toml",
                "--release",
                "--target-dir",
                "target/pg-language-server",
            ]))?;
    }
    require(&pg, "run mise run build-lsps")?;
    println!("Pinned language servers ready (existing binaries reused).");
    Ok(())
}
fn metadata(manifest: &Path) -> Result<Value> {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "Cargo metadata failed for {}: {}",
            manifest.display(),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}
fn registry_manifest(root: &Path) -> Result<PathBuf> {
    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).expect("user home")).join(".cargo"));
    let registries = cargo_home.join("registry/src");
    for entry in fs::read_dir(registries)? {
        let candidate = entry?
            .path()
            .join(format!("syntaqlite-cli-{SQLITE_VERSION}/Cargo.toml"));
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!("Pinned syntaqlite Cargo sources unavailable. Run mise run build-lsps from {} before generating notices.", root.display()).into())
}
fn is_notice(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    [
        "LICENSE",
        "LICENCE",
        "COPYING",
        "COPYRIGHT",
        "NOTICE",
        "UNLICENSE",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}
fn source_notice_files(package: &Value) -> Result<Vec<PathBuf>> {
    let manifest = PathBuf::from(
        package["manifest_path"]
            .as_str()
            .ok_or("Missing package manifest path")?,
    );
    let directory = manifest.parent().ok_or("Missing package directory")?;
    let mut roots = vec![directory.to_owned()];
    // Git workspace packages inherit notices from their checkout root. Only walk
    // bounded ancestors inside the exact Cargo checkout named by metadata.
    if package["source"]
        .as_str()
        .is_some_and(|source| source.starts_with("git+"))
    {
        for ancestor in directory.ancestors().skip(1).take(6) {
            if ancestor
                .parent()
                .is_some_and(|parent| parent.file_name().is_some_and(|name| name == "checkouts"))
            {
                break;
            }
            roots.push(ancestor.to_owned());
        }
    }
    let mut files = Vec::new();
    if let Some(file) = package["license_file"].as_str() {
        let path = directory.join(file);
        if path.is_file() {
            files.push(path);
        }
    }
    for directory in roots {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_type()?.is_file() && is_notice(&entry.file_name().to_string_lossy()) {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}
fn font_notices(root: &Path, destination: &Path) -> Result<()> {
    let target = destination.join("NotoEmoji");
    fs::create_dir_all(&target)?;
    for name in ["NotoEmoji-OFL.txt", "README.md"] {
        fs::copy(root.join("assets/fonts").join(name), target.join(name))?;
    }
    let inter = destination.join("Inter");
    fs::create_dir_all(&inter)?;
    for name in ["Inter-OFL.txt", "README.md"] {
        fs::copy(root.join("assets/fonts").join(name), inter.join(name))?;
    }
    Ok(())
}
fn icon_notices(root: &Path, destination: &Path) -> Result<()> {
    for (folder, name) in [("lucide", "Lucide"), ("databases", "Devicon")] {
        let target = destination.join(name);
        fs::create_dir_all(&target)?;
        for file in ["LICENSE", "README.md"] {
            fs::copy(root.join("assets/icons").join(folder).join(file), target.join(file))?;
        }
    }
    Ok(())
}
fn notices(root: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    font_notices(root, destination)?;
    icon_notices(root, destination)?;
    copy_tree(&root.join("assets/redis"), &destination.join("redis-command-metadata"))?;
    for name in [
        "UPSTREAM.md",
        "postgres-language-server-0.28.1-MIT.txt",
        "syntaqlite-0.13.3-Apache-2.0-and-SQLite.txt",
    ] {
        fs::copy(
            root.join("packaging/licenses").join(name),
            destination.join(name),
        )?;
    }
    let manifests = [
        root.join("Cargo.toml"),
        root.join("tools/postgres-language-server/Cargo.toml"),
        registry_manifest(root)?,
    ];
    let mut packages = BTreeMap::new();
    for manifest in manifests {
        for package in metadata(&manifest)?["packages"]
            .as_array()
            .ok_or("No metadata packages")?
        {
            packages.insert(
                package["id"]
                    .as_str()
                    .ok_or("Missing package ID")?
                    .to_owned(),
                package.clone(),
            );
        }
    }
    let mut inventory = Vec::new();
    for (index, (id, package)) in packages.into_iter().enumerate() {
        let name = package["name"].as_str().unwrap_or("unknown");
        let version = package["version"].as_str().unwrap_or("unknown");
        let relative = format!("dependencies/{name}-{version}-{index}");
        let files = source_notice_files(&package)?;
        let mut copied = Vec::new();
        for (file_index, file) in files.into_iter().enumerate() {
            let filename = format!(
                "{file_index}-{}",
                file.file_name().unwrap().to_string_lossy()
            );
            let target = destination.join(&relative);
            fs::create_dir_all(&target)?;
            fs::copy(&file, target.join(&filename))?;
            copied.push(format!("{relative}/{filename}"));
        }
        inventory.push(json!({"package": id, "name":name,"version":version,"license":package["license"],"source":package["source"],"notices":copied}));
    }
    fs::write(
        destination.join("cargo-dependencies.json"),
        serde_json::to_vec_pretty(&inventory)?,
    )?;
    Ok(())
}
fn bundle(root: &Path, binary: &Path, destination: &Path, plist: &Path) -> Result<()> {
    let sqlite = root.join(format!("target/lsp-tools/bin/syntaqlite{}", env::consts::EXE_SUFFIX));
    let pg = root.join(format!("target/pg-language-server/release/rustable-postgres-language-server{}", env::consts::EXE_SUFFIX));
    let icon = root.join("assets/Rustable.icns");
    // Validate everything before touching an existing application bundle.
    for (path, reason) in [
        (binary, "build Rustable first"),
        (sqlite.as_path(), "run mise run build-lsps"),
        (pg.as_path(), "run mise run build-lsps"),
        (plist, "provide a valid Info.plist"),
        (icon.as_path(), "provide the Rustable application icon"),
    ] {
        require(path, reason)?;
    }
    if !version_matches(&sqlite, SQLITE_VERSION) {
        return Err(
            "Bundled syntaqlite version mismatch; expected 0.13.3. Run mise run build-lsps.".into(),
        );
    }
    let license_dir = root.join("target/bundle-licenses");
    notices(root, &license_dir)?;
    let contents = destination.join("Contents");
    fs::create_dir_all(contents.join("MacOS"))?;
    fs::create_dir_all(contents.join("Resources"))?;
    copy_atomic(binary, &contents.join("MacOS/rustable"))?;
    copy_atomic(&sqlite, &contents.join("MacOS/syntaqlite"))?;
    fs::copy(
        &pg,
        contents.join("MacOS/rustable-postgres-language-server"),
    )?;
    copy_atomic(plist, &contents.join("Info.plist"))?;
    copy_atomic(&icon, &contents.join("Resources/Rustable.icns"))?;
    copy_tree(&license_dir, &contents.join("Resources/Licenses"))?;
    println!(
        "Bundled Rustable and both required language servers: {}",
        destination.display()
    );
    Ok(())
}
fn copy_atomic(source: &Path, destination: &Path) -> Result<()> {
    if source
        .canonicalize()
        .ok()
        .zip(destination.canonicalize().ok())
        .is_some_and(|(source, destination)| source == destination)
    {
        return Ok(());
    }
    let name = destination
        .file_name()
        .ok_or("Destination filename missing")?
        .to_string_lossy();
    let temporary =
        destination.with_file_name(format!(".{name}.replacement-{}", std::process::id()));
    fs::copy(source, &temporary)?;
    if let Err(error) = fs::rename(&temporary, destination) {
        let _ = fs::remove_file(temporary);
        return Err(error.into());
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    let root = root();
    match args.first().map(String::as_str) {
        Some("configure-updates") if args.len() == 5 => updates::configure(Path::new(&args[1]), Path::new(&args[2]), &args[3], &args[4]),
        Some("build-lsps") => build_servers(&root),
        Some("licenses") => notices(&root, &root.join("target/bundle-licenses")),
        Some("bundle") => bundle(&root, &args.get(1).map(PathBuf::from).unwrap_or(root.join("target/release/rustable")), &args.get(2).map(PathBuf::from).unwrap_or(root.join("target/Rustable.app")), &args.get(3).map(PathBuf::from).unwrap_or(root.join("packaging/macos/Info.plist"))),
        _ => Err("Usage: rustable-package build-lsps | licenses | bundle [app-binary] [output.app] [Info.plist] | configure-updates <app> <Sparkle.framework> <https-feed> <public-key>".into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn font_license_and_provenance_are_packaged_without_font_binary() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let destination = env::temp_dir().join(format!("rustable-font-notices-{}", std::process::id()));
        font_notices(root, &destination).unwrap();
        for name in ["NotoEmoji-OFL.txt", "README.md"] {
            assert_eq!(fs::read(destination.join("NotoEmoji").join(name)).unwrap(), fs::read(root.join("assets/fonts").join(name)).unwrap());
        }
        assert!(!destination.join("NotoEmoji/NotoEmoji.ttf").exists());
        assert_eq!(fs::read(destination.join("Inter/Inter-OFL.txt")).unwrap(), fs::read(root.join("assets/fonts/Inter-OFL.txt")).unwrap());
        fs::remove_dir_all(destination).unwrap();
    }

    #[test]
    fn missing_inputs_fail_before_bundle_mutation() {
        let base = env::temp_dir().join(format!("rustable-package-test-{}", std::process::id()));
        let destination = base.join("Existing.app");
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("keep"), "existing").unwrap();
        assert!(
            bundle(
                &base,
                &base.join("missing"),
                &destination,
                &base.join("Info.plist")
            )
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(destination.join("keep")).unwrap(),
            "existing"
        );
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn atomic_replacement_preserves_existing_open_inode_and_same_path_is_safe() {
        use std::io::Read;
        let base = env::temp_dir().join(format!("rustable-package-atomic-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let source = base.join("new");
        let destination = base.join("running");
        fs::write(&source, "new executable").unwrap();
        fs::write(&destination, "old executable").unwrap();
        let mut old = fs::File::open(&destination).unwrap();
        copy_atomic(&source, &destination).unwrap();
        let mut original = String::new();
        old.read_to_string(&mut original).unwrap();
        assert_eq!(original, "old executable");
        assert_eq!(fs::read_to_string(&destination).unwrap(), "new executable");
        copy_atomic(&destination, &destination).unwrap();
        assert_eq!(fs::read_to_string(&destination).unwrap(), "new executable");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn only_explicit_notice_names_match() {
        assert!(is_notice("LICENSE-MIT"));
        assert!(is_notice("COPYING"));
        assert!(!is_notice("README.md"));
        assert!(!is_notice("private.key"));
    }
}
