#![allow(dead_code)]

use suprnova::live::{
    AuthorizedAction, EffectPayloadMetadata, EventPayloadMetadata, LiveComponent, live,
};

struct Saved;

impl EventPayloadMetadata for Saved {
    const NAME: &'static str = "saved";
    const VERSION: u16 = 2;
}

struct Focus;

impl EffectPayloadMetadata for Focus {
    const NAME: &'static str = "focus";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "catalog.search",
    view = "live/catalog/search.html",
    component_version = 2,
    state_schema_version = 3,
    action_schema_version = 4,
    checker_contract_version = 5,
    minimum_protocol_version = 2,
    refresh_on_promote,
    events(Saved),
    effects(Focus)
)]
pub struct Search {
    title: String,
    #[public]
    prompt: String,
    #[model(debounce = 250)]
    #[url(key = "q", mode = "reflect", omit_default)]
    query: String,
    #[model(transient)]
    upload_token: String,
    #[locked]
    owner_id: i64,
    #[server_only]
    database_handle: String,
    #[session]
    locale: String,
    #[secret]
    csrf_secret: Vec<u8>,
}

#[live]
impl Search {
    #[mount]
    pub fn mount() -> Self {
        Self {
            title: String::new(),
            prompt: String::new(),
            query: String::new(),
            upload_token: String::new(),
            owner_id: 0,
            database_handle: String::new(),
            locale: String::new(),
            csrf_secret: Vec::new(),
        }
    }

    #[action(
        name = "save",
        version = 2,
        authorize = "current",
        validate = "all",
        transaction = "required"
    )]
    pub async fn persist(
        &mut self,
        _authorization: &AuthorizedAction,
        _title: String,
    ) {
    }

    #[computed]
    pub fn summary(&self) -> String {
        String::new()
    }

    #[validate]
    pub fn validate_search(&self) {}

    #[hydrate]
    pub fn hydrate(&mut self) {}

    #[rendering]
    pub fn rendering(&mut self) {}

    #[rendered]
    pub fn rendered(&mut self) {}

    #[dehydrate]
    pub fn dehydrate(&mut self) {}

    #[teardown]
    pub fn teardown(&mut self) {}

    #[params_changed]
    pub async fn params_changed(&mut self) {}

    #[lazy_complete]
    pub async fn lazy_complete(&mut self) {}
}

fn main() {
    let descriptor = <Search as ::suprnova::live::__private::metadata::LiveComponentContract>::descriptor()
        .expect("generated metadata must be valid");
    let metadata = descriptor.metadata();
    assert_eq!(metadata.identity().as_str(), "catalog.search");
    assert_eq!(metadata.view().as_str(), "live/catalog/search.html");
    assert_eq!(metadata.versions().component(), 2);
    assert_eq!(metadata.fields().len(), 8);
    assert_eq!(metadata.actions().len(), 1);
    assert_eq!(
        metadata.actions()[0].authorization(),
        ::suprnova::live::__private::action::AuthorizationRequirement::Current
    );
    assert_eq!(
        metadata.actions()[0].transaction(),
        ::suprnova::live::__private::action::TransactionPolicy::Required
    );
    assert!(metadata.refresh_on_promote());
}
