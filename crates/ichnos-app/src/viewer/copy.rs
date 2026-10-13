//! Copying from the viewer: a row, a text block, or a reference an agent can look up.

use std::time::Instant;

use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::{
    AnyElement, ClipboardItem, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, div,
};
use ichnos_model::call::Key;

use super::{COPY_ROW, FOR_AGENT, Viewer};
use crate::present::pretty;
use crate::view_state::agent_reference;

impl Viewer {
    pub(super) fn copy(
        &mut self,
        what: impl Into<SharedString>,
        text: String,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = Some((what.into(), Instant::now()));
        cx.notify();
    }

    /// Copy event `row_id` of the shown session as an `/ichnos` reference to give an agent.
    pub(super) fn copy_for_agent(&mut self, row_id: i64, cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            let text = agent_reference(session, row_id);
            self.copy(FOR_AGENT, text, cx);
        }
    }

    /// Copy row `key`'s events as pretty JSON, as the inspector's "raw" does.
    fn copy_row(&mut self, key: &Key, cx: &mut Context<Self>) {
        let events = match self.model.find(key) {
            Some(id) => self.model.node(id).raw_data(),
            None => self
                .table
                .iter()
                .find(|row| row.call.key() == key)
                .map(|row| {
                    row.call
                        .events()
                        .iter()
                        .map(|event| serde_json::from_str(event.json()).unwrap_or_default())
                        .collect()
                })
                .unwrap_or_default(),
        };
        self.copy(COPY_ROW, pretty(&serde_json::Value::Array(events)), cx);
    }

    /// `row` with a right-click menu: "Copy" copies row `key`'s events, and "Copy for agent"
    /// copies event `row_id` for an agent when the row names a single event.
    pub(super) fn with_row_menu(
        row: impl IntoElement + 'static,
        key: Key,
        row_id: Option<i64>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let viewer = cx.entity().downgrade();
        // Its own id per row: without one every row shares the same menu state.
        div()
            .id(SharedString::from(format!("row-menu-{key}")))
            .w_full()
            .child(row)
            .context_menu(move |menu, _, _| {
                let (copy, for_agent, key) = (viewer.clone(), viewer.clone(), key.clone());
                let menu = menu.item(PopupMenuItem::new("Copy").on_click(move |_, _, cx| {
                    copy.update(cx, |this, cx| this.copy_row(&key, cx)).ok();
                }));
                let Some(row_id) = row_id else {
                    return menu;
                };
                menu.item(
                    PopupMenuItem::new("Copy for agent").on_click(move |_, _, cx| {
                        for_agent
                            .update(cx, |this, cx| this.copy_for_agent(row_id, cx))
                            .ok();
                    }),
                )
            })
            .into_any_element()
    }

    pub(super) fn is_copied(&self, what: &str) -> bool {
        self.copied
            .as_ref()
            .is_some_and(|(copied, _)| copied == what)
    }
}
