use gtk::gdk;
use std::path::PathBuf;

pub(crate) fn configure_app_icons(app_id: &str) {
    if let Some(display) = gdk::Display::default() {
        let icon_theme = gtk::IconTheme::for_display(&display);
        icon_theme.add_search_path(bundled_icon_search_path());
    }
    gtk::Window::set_default_icon_name(app_id);
}

pub(crate) fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&exaterm_ui::css::generate_application_css());

    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display should exist"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn bundled_icon_search_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons")
}
