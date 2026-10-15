use suprnova::live::{EventPayloadMetadata, LiveComponent};

struct OrdersUpdated;

impl EventPayloadMetadata for OrdersUpdated {
    const NAME: &'static str = "orders.updated";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "orders.unknown",
    view = "live/example.html",
    minimum_protocol_version = 2,
    streams(stream(name = "orders", topics("orders"), events(OrdersUpdated), channel = "orders"))
)]
pub struct UnknownStreamHelper {
    value: String,
}

fn main() {}
