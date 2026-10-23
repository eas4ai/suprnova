use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

impl tracing::Subscriber for Warnings {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() == tracing::Level::WARN
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        struct Text(String);
        impl tracing::field::Visit for Text {
            fn record_debug(&mut self, _: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.push_str(&format!("{value:?}"));
            }
        }
        let mut text = Text(String::new());
        event.record(&mut text);
        self.0.lock().expect("warning capture").push(text.0);
    }
}

/// Boot the real registry under a subscriber so schema budgets are checked.
pub fn assert_quiet(router: &suprnova::Router) {
    let warnings = Warnings::default();
    tracing::subscriber::with_default(warnings.clone(), || {
        suprnova::agents::ToolRegistry::build(router).expect("the shipped tools boot");
    });
    let warnings = warnings.0.lock().expect("captured warnings");
    assert!(
        warnings.iter().all(|warning| !warning.contains("WebMCP")),
        "shipped tools exceeded a budget: {warnings:?}"
    );
}
