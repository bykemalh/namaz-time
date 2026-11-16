mod application;
mod config;
mod i18n;
mod models;
mod providers;
mod services;
mod ui;
mod window;

use self::application::NamazTimeApplication;
use config::PKGDATADIR;
use gtk::prelude::*;
use gtk::{gio, glib};
use tracing_subscriber::EnvFilter;

fn main() -> glib::ExitCode {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("namaz_time=info,warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();

    i18n::init();

    let installed_res = format!("{PKGDATADIR}/namaz-time.gresource");
    let resources = gio::Resource::load(&installed_res)
        .or_else(|_| gio::Resource::load("_build/src/namaz-time.gresource"))
        .expect("Could not load namaz-time.gresource");
    gio::resources_register(&resources);

    let app = NamazTimeApplication::new("io.github.bykemalh.namaz_time.NamazTime", &gio::ApplicationFlags::empty());
    let exit_code = app.run();
    drop(app);

    if i18n::take_restart_requested() {
        if let Ok(exe) = std::env::current_exe() {
            use std::os::unix::process::CommandExt;
            let err = std::process::Command::new(exe).exec();
            tracing::error!("Failed to restart application: {err}");
        }
    }

    exit_code
}
