use gpui::*;
use gpui_component::{input::InputState, v_flex, ActiveTheme};
use kaze_shared::FileEntry;

use crate::SearchModel;

pub struct SearchView {
    pub query: Entity<InputState>,
    pub results: Vec<FileEntry>,
    model: std::sync::Arc<SearchModel>,
}

impl SearchView {
    pub fn new(
        model: std::sync::Arc<SearchModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search files…"));

        cx.observe(&query, |this, query, cx| {
            let text = query.read(cx).value().to_string();
            if text.is_empty() {
                this.results.clear();
                cx.notify();
            } else {
                let model = this.model.clone();
                cx.spawn(async move |this, cx| {
                    // Debounce: wait 150ms before searching
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(150))
                        .await;
                    let results = cx
                        .background_executor()
                        .spawn(async move { model.search(&text, 100) })
                        .await;
                    this.update(cx, |this, cx| {
                        this.results = results;
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
        })
        .detach();

        Self {
            query,
            results: vec![],
            model,
        }
    }
}

impl Render for SearchView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .gap_2()
            .size_full()
            .px_3()
            .py_2()
            .child(self.query.clone())
            .children(self.results.iter().take(50).map(|entry| {
                div()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .hover(|this| this.bg(theme.muted))
                    .child(entry.name.clone())
            }))
    }
}
