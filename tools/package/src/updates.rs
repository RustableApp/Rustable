//! Prepare a Sparkle-enabled bundle before Developer ID signing/notarization.
use crate::{Result, require, run};
use base64::Engine;
use std::{path::Path, process::Command};

fn validate(feed: &str, key: &str) -> Result<()> {
    let url = url::Url::parse(feed)?;
    if url.scheme() != "https" || url.host_str().is_none()
        || !url.username().is_empty() || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("Update feed must be an HTTPS URL without credentials or fragment".into());
    }
    let decoded = base64::engine::general_purpose::STANDARD.decode(key)?;
    if decoded.len() != 32 || decoded.iter().all(|byte| *byte == 0) {
        return Err("Sparkle public Ed25519 key must contain 32 nonzero-key bytes".into());
    }
    Ok(())
}

pub fn configure(bundle: &Path, framework: &Path, feed: &str, key: &str) -> Result<()> {
    validate(feed, key)?;
    require(&framework.join("Sparkle"), "provide the official Sparkle.framework")?;
    let license = framework.parent().ok_or("Missing Sparkle distribution directory")?.join("LICENSE");
    require(&license, "provide Sparkle's distribution LICENSE alongside its framework")?;
    let binary = bundle.join("Contents/MacOS/rustable");
    require(&binary, "build and bundle with --features sparkle-updater first")?;
    let linked = Command::new("otool").arg("-L").arg(&binary).output()?;
    if !linked.status.success() || !String::from_utf8_lossy(&linked.stdout).contains("Sparkle.framework/") {
        return Err("This app binary is not linked to Sparkle; build with --features sparkle-updater".into());
    }
    let plist_path = bundle.join("Contents/Info.plist");
    let mut plist = plist::Value::from_file(&plist_path)?;
    let dict = plist.as_dictionary_mut().ok_or("Info.plist must be a dictionary")?;
    if dict.get("CFBundleIdentifier").and_then(plist::Value::as_string) != Some("dev.rustable.app") {
        return Err("Public updates can only be configured for the production Rustable bundle".into());
    }
    dict.insert("SUFeedURL".into(), feed.into());
    dict.insert("SUPublicEDKey".into(), key.into());
    // Ask users about scheduled checks through Sparkle's native permission UI.
    dict.insert("SUEnableInstallerLauncherService".into(), true.into());
    dict.insert("SUAutomaticallyUpdate".into(), false.into());
    // ditto preserves the framework's versioned symlinks and executable modes.
    run(Command::new("ditto").arg(framework).arg(bundle.join("Contents/Frameworks/Sparkle.framework")))?;
    let notices = bundle.join("Contents/Resources/Licenses/Sparkle");
    std::fs::create_dir_all(&notices)?;
    std::fs::copy(&license, notices.join("LICENSE"))?;
    plist.to_file_xml(&plist_path)?;
    println!("Sparkle configured. Sign nested components and app, notarize, then sign/publish the update archive.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_insecure_feeds_and_invalid_public_keys() {
        let key = base64::engine::general_purpose::STANDARD.encode([7; 32]);
        assert!(validate("https://updates.example.com/macos/appcast.xml", &key).is_ok());
        for feed in ["http://updates.example.com/feed", "file:///tmp/feed", "https://user:password@example.com/feed", "https://example.com/feed#fragment"] {
            assert!(validate(feed, &key).is_err(), "{feed}");
        }
        for key in ["".to_string(), "private-key".into(), base64::engine::general_purpose::STANDARD.encode([0;32]), base64::engine::general_purpose::STANDARD.encode([1;64])] {
            assert!(validate("https://example.com/feed", &key).is_err());
        }
    }
}
