use gpui::*;
use gpui_component::{input::InputState, v_flex, ActiveTheme};
use kaze_shared::FileEntry;

use crate::SearchModel;

pub struct SearchView {
    pub query: Entity<InputState>,
    pub results: Vec<FileEntry>,
    pub searching: bool,
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
                this.searching = false;
                cx.notify();
            } else {
                // Immediate feedback: mark searching right away
                this.searching = true;
                cx.notify();
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
                        this.searching = false;
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
            searching: false,
            model,
        }
    }
}

impl Render for SearchView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let query_text = self.query.read(cx).value().to_string();
        let is_searching = self.searching;
        let has_query = !query_text.is_empty();
        let has_results = !self.results.is_empty();

        v_flex()
            .gap_2()
            .size_full()
            .px_3()
            .py_2()
            .child(self.query.clone())
            .when(is_searching, |this| {
                this.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("Searching…"),
                )
            })
            .when(!is_searching && has_query && !has_results, |this| {
                this.child(
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(
                            gpui_component::Icon::new(gpui_component::IconName::Search)
                                .large()
                                .text_color(theme.muted_foreground.alpha(0.4)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(format!("No results for “{}”.", query_text)),
                        ),
                )
            })
            .children(self.results.iter().take(50).map(|entry| {
                div()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .hover(|this| this.bg(theme.muted.alpha(0.08)))
                    .child(entry.name.clone())
            }))
    }
}
