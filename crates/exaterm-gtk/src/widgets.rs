use std::{cell::RefCell, rc::Rc};

use gtk::prelude::*;
use vte4 as vte;

#[derive(Clone)]
pub(crate) struct SegmentedBarWidgets {
    pub frame: gtk::Box,
    pub reason: gtk::Label,
    pub segments: Vec<gtk::Box>,
}

#[derive(Clone)]
pub(crate) struct SessionCardWidgets {
    pub row: gtk::FlowBoxChild,
    pub frame: gtk::Frame,
    pub header: gtk::Box,
    pub title: gtk::Label,
    pub status: gtk::Label,
    pub headline_row: gtk::Box,
    pub attention_pill: gtk::Label,
    pub nudge_row: gtk::Box,
    pub nudge_state: gtk::Label,
    pub recency: gtk::Label,
    pub middle_stack: gtk::Stack,
    pub scrollback_band: gtk::Frame,
    pub scrollback_content: gtk::DrawingArea,
    pub scrollback_lines: Rc<RefCell<Vec<String>>>,
    pub terminal_slot: gtk::Box,
    pub footer: gtk::Box,
    pub bars: gtk::Box,
    pub headline: gtk::Label,
    pub alert: gtk::Label,
    pub momentum_bar: SegmentedBarWidgets,
    pub risk_bar: SegmentedBarWidgets,
    pub terminal_view: gtk::ScrolledWindow,
    pub terminal: vte::Terminal,
}

pub(crate) struct FocusWidgets {
    pub panel: gtk::Box,
    pub frame: gtk::Frame,
    pub header: gtk::Box,
    pub title: gtk::Label,
    pub status: gtk::Label,
    pub summary_box: gtk::Box,
    pub headline: gtk::Label,
    pub attention_pill: gtk::Label,
    pub alert: gtk::Label,
    pub terminal_slot: gtk::Box,
    pub bars: gtk::Box,
    pub momentum_bar: SegmentedBarWidgets,
    pub risk_bar: SegmentedBarWidgets,
}

pub(crate) fn build_segmented_bar(label: &str) -> SegmentedBarWidgets {
    let caption = gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .css_classes(vec!["bar-caption".to_string()])
        .build();
    let bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(4)
        .hexpand(true)
        .build();
    bar.add_css_class("segmented-bar");
    let segments = (0..5)
        .map(|_| {
            let segment = gtk::Box::builder().hexpand(true).build();
            segment.add_css_class("bar-segment");
            bar.append(&segment);
            segment
        })
        .collect::<Vec<_>>();
    let reason = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["bar-reason".to_string()])
        .build();
    reason.set_lines(3);
    reason.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let frame = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .build();
    frame.add_css_class("bar-widget");
    frame.set_halign(gtk::Align::Fill);
    frame.append(&caption);
    frame.append(&bar);
    frame.append(&reason);
    SegmentedBarWidgets {
        frame,
        reason,
        segments,
    }
}
