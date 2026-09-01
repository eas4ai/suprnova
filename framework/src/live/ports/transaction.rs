//! Suprnova database transaction adaptation for Live execution.

use suprnova_live::component::LiveFuture;
use suprnova_live::execution::{HostError, HostErrorKind, HostTransaction, TransactionPort};

pub(crate) struct SuprnovaTransactionPort;

struct SuprnovaHostTransaction(crate::database::Transaction);

impl HostTransaction for SuprnovaHostTransaction {
    fn commit(self: Box<Self>) -> LiveFuture<'static, Result<(), HostError>> {
        Box::pin(async move {
            self.0
                .commit()
                .await
                .map_err(|_| HostError::new(HostErrorKind::Commit))
        })
    }

    fn rollback(self: Box<Self>) -> LiveFuture<'static, Result<(), HostError>> {
        Box::pin(async move {
            self.0
                .rollback()
                .await
                .map_err(|_| HostError::new(HostErrorKind::Rollback))
        })
    }
}

impl TransactionPort for SuprnovaTransactionPort {
    fn begin(&self) -> LiveFuture<'_, Result<Box<dyn HostTransaction>, HostError>> {
        Box::pin(async {
            crate::database::DB::begin_transaction()
                .await
                .map(|transaction| {
                    Box::new(SuprnovaHostTransaction(transaction)) as Box<dyn HostTransaction>
                })
                .map_err(|_| HostError::new(HostErrorKind::Begin))
        })
    }
}
