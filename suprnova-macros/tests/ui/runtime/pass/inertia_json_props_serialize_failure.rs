//! A JSON-style prop whose `Serialize` fails is an error the handler
//! returns, not a panic, the same as a typed prop.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use suprnova::serde::ser::Error as _;
use suprnova::serde::{Serialize, Serializer};
use suprnova::{FrameworkError, HttpResponse, InertiaRequestExt, inertia_response};

struct Boom;

impl Serialize for Boom {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        Err(S::Error::custom("boom"))
    }
}

struct Probe;

impl InertiaRequestExt for Probe {
    fn path(&self) -> &str {
        "/"
    }

    fn header(&self, _name: &str) -> Option<&str> {
        None
    }
}

async fn page(request: &Probe) -> Result<HttpResponse, FrameworkError> {
    let label = "kept";
    inertia_response!(request, "Probe", {
        "label": label,
        "items": [1, null, { "deep": true }],
        "nested": { "bad": Boom },
    })
}

fn main() {
    let request = Probe;
    let mut future = pin!(page(&request));
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(Err(error)) => {
            let message = error.to_string();
            assert!(message.contains("nested"), "names the prop: {message}");
            assert!(message.contains("boom"), "carries the cause: {message}");
        }
        Poll::Ready(Ok(_)) => panic!("a prop that cannot serialize must not render"),
        Poll::Pending => panic!("the failure is reported before any await"),
    }
}
