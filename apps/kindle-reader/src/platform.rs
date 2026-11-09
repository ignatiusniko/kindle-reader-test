//! Display backend selection. The Kindle adapter is replaceable (spec §4
//! `platform`): everything device-specific stays in this file.

use crate::config::Config;

pub enum Backend {
    #[cfg(feature = "kindle")]
    Kindle(#[allow(dead_code)] slint_backend_kindle::KindleBackend),
    #[cfg(feature = "desktop")]
    Desktop,
}

#[cfg(feature = "kindle")]
static FONT: &[u8] = include_bytes!("../../../assets/fonts/ReaderFont.ttf");

pub fn install(cfg: &Config) -> Result<Backend, String> {
    #[cfg(feature = "kindle")]
    if !cfg.desktop {
        // May fail on KT5 until the backend's MTK path is verified (G4).
        let backend = slint_backend_kindle::install(FONT).map_err(|e| e.to_string())?;
        // Manga needs grayscale; never force bilevel for the page.
        backend.set_black_and_white(false);
        return Ok(Backend::Kindle(backend));
    }
    #[cfg(feature = "desktop")]
    {
        let _ = cfg;
        return Ok(Backend::Desktop);
    }
    #[allow(unreachable_code)]
    {
        let _ = cfg;
        Err(
            "this build has no display backend: rebuild with `--features kindle` or \
             `--features desktop`, or use the `snapshot` command"
                .into(),
        )
    }
}
