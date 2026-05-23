use vte4 as vte;

#[derive(Clone)]
pub(crate) struct SessionCardWidgets {
    pub frame: gtk::Frame,
    pub terminal_slot: gtk::Box,
    pub terminal_overlay: gtk::Overlay,
    pub terminal_dim_overlay: gtk::Box,
    pub terminal_assist_overlay: gtk::Box,
    pub terminal_assist_entry: gtk::Entry,
    pub terminal_assist_status: gtk::Label,
    pub terminal_assist_spinner: gtk::Spinner,
    pub terminal_assist_cancel: gtk::Button,
    pub terminal: vte::Terminal,
}

#[derive(Clone)]
pub(crate) struct GroupCardWidgets {
    pub frame: gtk::Frame,
    pub title: gtk::Label,
    pub subtitle: gtk::Label,
    pub status: gtk::Label,
    pub summary_content: gtk::Box,
    pub rendered_summary_markdown: std::rc::Rc<std::cell::RefCell<String>>,
    pub supervisor_toggle: gtk::ToggleButton,
    pub supervisor_toggle_updating: std::rc::Rc<std::cell::Cell<bool>>,
    pub middle_stack: gtk::Stack,
    pub summary_view: gtk::ScrolledWindow,
    pub terminal_slot: gtk::Box,
}
