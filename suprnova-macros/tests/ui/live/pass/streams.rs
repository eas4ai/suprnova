#![allow(dead_code)]

use suprnova::live::{EventPayloadMetadata, LiveComponent, live};

struct OrdersUpdated;

impl EventPayloadMetadata for OrdersUpdated {
    const NAME: &'static str = "orders.updated";
    const VERSION: u16 = 1;
}

struct StockChanged;

impl EventPayloadMetadata for StockChanged {
    const NAME: &'static str = "stock.changed";
    const VERSION: u16 = 3;
}

#[derive(LiveComponent)]
#[live(
    name = "orders.list",
    view = "live/example.html",
    minimum_protocol_version = 2,
    streams(
        stream(
            name = "orders",
            topics("orders/:tenant", "orders"),
            events(OrdersUpdated),
            targets("self", "document"),
            fanout = 4,
            modes("sse", "websocket"),
            reconnect = "resume_or_refresh",
            resume_attempts = 6,
        ),
        stream(
            name = "inventory",
            topics("inventory"),
            events(StockChanged),
            modes("sse"),
            reconnect = "refresh_on_reconnect",
        ),
    )
)]
pub struct OrdersList {
    #[model]
    filter: String,
}

#[live]
impl OrdersList {
    #[mount]
    pub fn mount() -> Self {
        Self {
            filter: String::new(),
        }
    }
}

fn main() {
    let descriptor =
        <OrdersList as ::suprnova::live::__private::metadata::LiveComponentContract>::descriptor()
            .expect("generated stream metadata must be valid");
    let metadata = descriptor.metadata();
    assert_eq!(metadata.subscriptions().len(), 2);

    let orders = metadata
        .subscriptions()
        .iter()
        .find(|subscription| subscription.stream().as_str() == "orders")
        .expect("orders stream is registered");
    assert_eq!(orders.topics().as_slice().len(), 2);
    assert_eq!(orders.events().as_slice().len(), 1);
    assert_eq!(orders.events().as_slice()[0].as_str(), "orders.updated");
    assert_eq!(orders.modes().as_slice().len(), 2);
    assert_eq!(
        orders.reconnect(),
        ::suprnova::live::__private::async_updates::ReconnectPolicy::ResumeOrRefresh {
            maximum_attempts: ::core::num::NonZeroU8::new(6).expect("six attempts"),
        }
    );

    let inventory = metadata
        .subscriptions()
        .iter()
        .find(|subscription| subscription.stream().as_str() == "inventory")
        .expect("inventory stream is registered");
    assert_eq!(inventory.modes().as_slice().len(), 1);
    assert_eq!(
        inventory.reconnect(),
        ::suprnova::live::__private::async_updates::ReconnectPolicy::RefreshOnReconnect
    );

    let event = metadata
        .events()
        .iter()
        .find(|event| event.name().as_str() == "orders.updated")
        .expect("stream events join the component event contracts");
    assert_eq!(
        event.source(),
        ::suprnova::live::__private::async_updates::EventSource::Stream
    );
    assert_eq!(event.maximum_fanout().get(), 4);
    assert_eq!(event.targets().as_slice().len(), 2);
    assert_eq!(metadata.events().len(), 2);
}
