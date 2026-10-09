use suprnova::live::{EventPayloadMetadata, LiveComponent};

struct OrdersUpdated;

impl EventPayloadMetadata for OrdersUpdated {
    const NAME: &'static str = "orders.updated";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "orders.legacy",
    view = "live/example.html",
    streams(stream(name = "orders", topics("orders"), events(OrdersUpdated)))
)]
pub struct LegacyProtocolStream {
    value: String,
}

fn main() {}
