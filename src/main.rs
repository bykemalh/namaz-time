mod application;
mod config;
mod models;
mod providers;
mod services;
mod ui;
mod window;

use self::application::NamazTimeApplication;
use config::{GETTEXT_PACKAGE, LOCALEDIR, PKGDATADIR};
use gettextrs::{bind_textdomain_codeset, bindtextdomain, textdomain};
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

    bindtextdomain(GETTEXT_PACKAGE, LOCALEDIR).expect("Unable to bind the text domain");
    bind_textdomain_codeset(GETTEXT_PACKAGE, "UTF-8")
        .expect("Unable to set the text domain encoding");
    textdomain(GETTEXT_PACKAGE).expect("Unable to switch to the text domain");

    let installed_res = format!("{PKGDATADIR}/namaz-time.gresource");
    let resources = gio::Resource::load(&installed_res)
        .or_else(|_| gio::Resource::load("_build/src/namaz-time.gresource"))
        .expect("Could not load namaz-time.gresource");
    gio::resources_register(&resources);

    let app = NamazTimeApplication::new("me.bykemah.namaztime", &gio::ApplicationFlags::empty());
    app.run()
}
