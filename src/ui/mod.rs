pub mod location_dialog;
pub mod preferences;

pub use location_dialog::show_location_dialog;
pub use preferences::show_preferences_dialog;

use adw::prelude::*;

pub fn set_css_class(widget: &impl IsA<gtk::Widget>, class_name: &str, active: bool) {
    let has = widget.has_css_class(class_name);
    if active && !has {
        widget.add_css_class(class_name);
    } else if !active && has {
        widget.remove_css_class(class_name);
    }
}
