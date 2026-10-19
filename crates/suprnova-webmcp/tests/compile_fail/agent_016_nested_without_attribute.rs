use serde::Deserialize;
use validator::Validate;

/// A line of an order, with the derives but not the attribute.
#[derive(Deserialize, schemars::JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
struct OrderLine {
    #[validate(range(min = 1))]
    unit_price: u32,
}

/// An order whose nested lines cannot name their fields.
#[suprnova_webmcp::tool_input]
struct PlaceOrder {
    #[validate(nested)]
    line_items: Vec<OrderLine>,
}

fn main() {}
