use crate::ui::{refresh_runtime_and_cards, update_nudge_widgets, AppContext, NudgeCacheEntry};
use exaterm_types::model::SessionId;
use exaterm_types::proto::ClientMessage;
use std::rc::Rc;

pub(crate) fn toggle_auto_nudge(context: &Rc<AppContext>, session_id: SessionId) {
    let enabled = {
        let mut cache = context.nudge_cache.borrow_mut();
        let entry = cache.entry(session_id).or_insert_with(NudgeCacheEntry::new);
        entry.enabled = !entry.enabled;
        if !entry.enabled {
            entry.hovered = false;
        }
        entry.enabled
    };
    if let Some(beachhead) = context.beachhead.as_ref() {
        let _ = beachhead.commands().send(ClientMessage::ToggleAutoNudge {
            session_id,
            enabled,
        });
    }
    update_nudge_widgets(context, session_id);
    if enabled {
        refresh_runtime_and_cards(context);
    }
}

pub(crate) fn set_auto_nudge_hover(context: &Rc<AppContext>, session_id: SessionId, hovered: bool) {
    let changed = {
        let mut cache = context.nudge_cache.borrow_mut();
        let entry = cache.entry(session_id).or_insert_with(NudgeCacheEntry::new);
        let changed = entry.hovered != hovered;
        entry.hovered = hovered;
        changed
    };
    if changed {
        update_nudge_widgets(context, session_id);
    }
}

