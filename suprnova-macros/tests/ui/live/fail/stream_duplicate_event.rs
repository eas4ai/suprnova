use suprnova::live::{EventPayloadMetadata, LiveComponent};

struct OrdersUpdated;

impl EventPayloadMetadata for OrdersUpdated {
    const NAME: &'static str = "orders.updated";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "orders.duplicate",
    view = "live/example.html",
    minimum_protocol_version = 2,
    streams(
        stream(name = "orders", topics("orders"), events(OrdersUpdated)),
        stream(name = "archive", topics("archive"), events(OrdersUpdated)),
    )
)]
pub struct DuplicateStreamEvent {
    value: String,
}

fn main() {}
