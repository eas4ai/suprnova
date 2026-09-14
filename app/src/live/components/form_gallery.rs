//! `app.form-gallery`: mounts every presentational form component the
//! library ships, so `live:check` and the document tests exercise the real
//! set (Cairn FORM-001).

use suprnova::live::{LiveComponent, live};

/// A form that uses each shipped presentational component once, rendered
/// by `live/form-gallery.html`.
#[derive(LiveComponent)]
#[live(name = "app.form-gallery", view = "live/form-gallery.html")]
pub struct FormGallery {
    /// Search text, bound with the library's debounced search input.
    #[model(debounce = 300)]
    query: String,
    /// Email address, bound with the plain input.
    #[model]
    email: String,
    /// Free text, bound with the textarea.
    #[model]
    bio: String,
    /// Seat count, bound with the number input.
    #[model]
    quantity: u64,
    /// Volume, bound with the slider.
    #[model]
    volume: u64,
    /// The password, transient so it never enters a snapshot (FORM-004).
    #[model(transient)]
    secret: String,
    /// Terms acceptance, bound with the checkbox.
    #[model]
    agree: bool,
    /// Chosen plan, bound with the radio group.
    #[model]
    plan: String,
    /// Newsletter opt-in, bound with the switch.
    #[model]
    newsletter: bool,
    /// Country, bound with the select.
    #[model]
    country: String,
    /// Topics, bound with the checkbox group.
    #[model]
    topics: Vec<String>,
    /// Plan choices offered to the radio group.
    #[public]
    plans: Vec<(String, String)>,
    /// Country choices offered to the select.
    #[public]
    countries: Vec<(String, String)>,
    /// Topic choices offered to the checkbox group.
    #[public]
    topic_options: Vec<(String, String)>,
}

#[live]
impl FormGallery {
    /// Starts every control empty with the fixed option lists.
    #[mount]
    pub fn mount() -> Self {
        Self {
            query: String::new(),
            email: String::new(),
            bio: String::new(),
            quantity: 1,
            volume: 50,
            secret: String::new(),
            agree: false,
            plan: String::new(),
            newsletter: false,
            country: String::new(),
            topics: Vec::new(),
            plans: vec![
                ("starter".to_owned(), "Starter".to_owned()),
                ("team".to_owned(), "Team".to_owned()),
            ],
            countries: vec![
                ("ca".to_owned(), "Canada".to_owned()),
                ("us".to_owned(), "United States".to_owned()),
            ],
            topic_options: vec![
                ("releases".to_owned(), "Releases".to_owned()),
                ("security".to_owned(), "Security advisories".to_owned()),
            ],
        }
    }

    /// Accepts the form; the gallery keeps what was entered.
    #[action]
    pub fn save(&mut self) {}

    /// Clears every control back to its mounted value.
    #[action]
    pub fn reset(&mut self) {
        let fresh = Self::mount();
        self.query = fresh.query;
        self.email = fresh.email;
        self.bio = fresh.bio;
        self.quantity = fresh.quantity;
        self.volume = fresh.volume;
        self.secret = fresh.secret;
        self.agree = fresh.agree;
        self.plan = fresh.plan;
        self.newsletter = fresh.newsletter;
        self.country = fresh.country;
        self.topics = fresh.topics;
    }
}
