use std::path::Path;

use ashpd::desktop::background::{Background, BackgroundProxy, SetStatusOptions};
use tracing::{debug, info, warn};

use crate::i18n::tr_fmt;

pub fn is_sandboxed() -> bool {
    Path::new("/.flatpak-info").exists()
}

pub async fn request_background(autostart: bool) -> Result<(bool, bool), String> {
    if !is_sandboxed() {
        debug!("Flatpak sandbox dışında çalışılıyor; Background portalı atlandı.");
        return Ok((true, autostart));
    }

    let response = Background::request()
        .reason(
            crate::i18n::tr(
                "Display prayer times and live countdown in the background and system tray",
            )
            .as_str(),
        )
        .auto_start(autostart)
        .dbus_activatable(true)
        .send()
        .await
        .map_err(|e| tr_fmt("Background portal request failed: %1", &[&e.to_string()]))?
        .response()
        .map_err(|e| tr_fmt("Background portal response failed: %1", &[&e.to_string()]))?;

    info!(
        "Background portal sonucu: background={}, autostart={}",
        response.run_in_background(),
        response.auto_start()
    );
    Ok((response.run_in_background(), response.auto_start()))
}

pub async fn set_background_status(message: &str) {
    if !is_sandboxed() {
        return;
    }

    let Ok(proxy) = BackgroundProxy::new().await else {
        return;
    };
    if proxy.version() < 2 {
        return;
    }
    if let Err(err) = proxy
        .set_status(SetStatusOptions::default().set_message(message))
        .await
    {
        warn!("Background portal durum mesajı güncellenemedi: {err}");
    }
}
