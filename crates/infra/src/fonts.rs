//! Operating system fonts shared by the graphical tools.

use std::path::PathBuf;

/// Reads an installed Korean font for registration with the application's GUI.
pub fn korean_fallback() -> Option<Vec<u8>> {
    let path = if cfg!(windows) {
        PathBuf::from(std::env::var_os("WINDIR")?).join("Fonts/malgun.ttf")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/System/Library/Fonts/AppleSDGothicNeo.ttc")
    } else {
        PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc")
    };
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "Korean font unavailable");
            None
        }
    }
}
