//! Build-script helper that embeds the Ferrent icon and a VERSIONINFO block
//! into the Windows executables.
//!
//! The resource is compiled by `windres` from mingw-w64, the same toolchain the
//! `*-windows-gnu` build already links with; a native MSVC build would need
//! `rc.exe` and is not supported.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Embeds `packaging/windows/ferrent.ico` and a VERSIONINFO block.
///
/// A no-op on non-Windows targets. A missing `windres` is a warning rather
/// than an error: `cargo check --target x86_64-pc-windows-gnu` links nothing,
/// and `packaging/windows/stage-artifacts.sh` gates the real binaries on the
/// resource section, so a silently icon-less release is caught in CI.
pub fn embed(description: &str) {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let Some(workspace) = manifest_dir.join("../..").canonicalize().ok() else {
        println!(
            "cargo:warning=cannot resolve the workspace root; the Windows icon is not embedded"
        );
        return;
    };
    let icon = workspace.join("packaging/windows/ferrent.ico");
    println!("cargo:rerun-if-changed={}", icon.display());

    let Some(windres) = windres() else {
        println!("cargo:warning=windres not found; the Windows icon is not embedded");
        return;
    };

    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let script = out_dir.join("ferrent-resource.rc");
    fs::write(&script, resource_script(&icon, &version, description)).expect("write the .rc");

    let object = out_dir.join("ferrent-resource.o");
    let status = Command::new(&windres)
        .arg("-I")
        .arg(icon.parent().expect("the icon has a directory"))
        .arg("-O")
        .arg("coff")
        .arg("-o")
        .arg(&object)
        .arg(&script)
        .status()
        .unwrap_or_else(|error| panic!("running {windres}: {error}"));
    assert!(status.success(), "windres failed on {}", script.display());

    println!("cargo:rustc-link-arg={}", object.display());
}

fn resource_script(icon: &Path, version: &str, description: &str) -> String {
    let [major, minor, patch, build] = version_quad(version);
    let icon = icon.display().to_string().replace('\\', "\\\\");
    format!(
        "1 ICON \"{icon}\"\n\
         1 VERSIONINFO\n\
         FILEVERSION {major},{minor},{patch},{build}\n\
         PRODUCTVERSION {major},{minor},{patch},{build}\n\
         FILEOS 0x40004\n\
         FILETYPE 0x1\n\
         BEGIN\n\
           BLOCK \"StringFileInfo\"\n\
           BEGIN\n\
             BLOCK \"040904B0\"\n\
             BEGIN\n\
               VALUE \"CompanyName\", \"Ferrent\"\n\
               VALUE \"FileDescription\", \"{description}\"\n\
               VALUE \"FileVersion\", \"{version}\"\n\
               VALUE \"LegalCopyright\", \"MIT OR Apache-2.0\"\n\
               VALUE \"ProductName\", \"Ferrent\"\n\
               VALUE \"ProductVersion\", \"{version}\"\n\
             END\n\
           END\n\
           BLOCK \"VarFileInfo\"\n\
           BEGIN\n\
             VALUE \"Translation\", 0x409, 1200\n\
           END\n\
         END\n"
    )
}

/// `VERSIONINFO` wants four numbers; `1.0.0-rc1` becomes `1,0,0,0`.
fn version_quad(version: &str) -> [u32; 4] {
    let numeric = version.split(['-', '+']).next().unwrap_or_default();
    let mut quad = [0u32; 4];
    for (slot, part) in quad.iter_mut().zip(numeric.split('.')) {
        *slot = part.parse().unwrap_or(0);
    }
    quad
}

fn windres() -> Option<String> {
    if let Ok(explicit) = env::var("WINDRES") {
        return Some(explicit);
    }
    let target = env::var("TARGET").unwrap_or_default();
    let arch = target.split('-').next().unwrap_or("x86_64");
    [format!("{arch}-w64-mingw32-windres"), "windres".to_owned()]
        .into_iter()
        .find(|candidate| Command::new(candidate).arg("--version").output().is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pre_release_version_becomes_a_numeric_quad() {
        assert_eq!(version_quad("1.0.0-rc1"), [1, 0, 0, 0]);
        assert_eq!(version_quad("2.3.4"), [2, 3, 4, 0]);
        assert_eq!(version_quad("0.1.2+build7"), [0, 1, 2, 0]);
        assert_eq!(version_quad("nonsense"), [0, 0, 0, 0]);
    }

    #[test]
    fn the_script_carries_the_icon_and_the_version() {
        let script = resource_script(
            Path::new("/repo/packaging/windows/ferrent.ico"),
            "1.0.0-rc1",
            "Ferrent CLI",
        );
        assert!(script.contains("1 ICON \"/repo/packaging/windows/ferrent.ico\""));
        assert!(script.contains("FILEVERSION 1,0,0,0"));
        assert!(script.contains("VALUE \"FileVersion\", \"1.0.0-rc1\""));
        assert!(script.contains("VALUE \"FileDescription\", \"Ferrent CLI\""));
    }
}
