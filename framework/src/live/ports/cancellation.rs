//! Request-lifetime bridge to the engine's advisory cancellation flag.

use suprnova_live::resource::CancellationFlag;

pub(crate) struct SuprnovaCancellationPort;

impl SuprnovaCancellationPort {
    pub(crate) fn attach(&self) -> LiveRequestCancellation {
        LiveRequestCancellation {
            flag: CancellationFlag::new(),
        }
    }
}

pub(crate) struct LiveRequestCancellation {
    flag: CancellationFlag,
}

impl LiveRequestCancellation {
    pub(crate) fn flag(&self) -> CancellationFlag {
        self.flag.clone()
    }
}

impl Drop for LiveRequestCancellation {
    fn drop(&mut self) {
        self.flag.cancel();
    }
}
