//! Deterministic host-service controls for component conformance tests.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use suprnova_live::action::{
    ActionAuthorizationPort, ActionAuthorizationRequest, ActionError, ActionFuture,
    AuthorizationDecision,
};
use suprnova_live::clock::{Clock, ClockError};
use suprnova_live::component::LiveFuture;
use suprnova_live::execution::{HostError, HostErrorKind, HostTransaction, TransactionPort};
use suprnova_live::identity::{InstanceId, ModelField, UnixMillis};
use suprnova_live::random::{InstanceIdGenerator, RandomError};
use suprnova_live::state::{
    SessionError, SessionField, SessionIntent, SessionIntentKind, SessionPort, SessionValue,
};
use suprnova_live::validation::{
    ValidationFuture, ValidationIssue, ValidationPort, ValidationPortError, ValidationRequest,
};

use crate::upload::ControlledUploadAuthorization;
use crate::{HarnessTrace, HarnessTraceEvent};

/// Deterministic wall clock whose value and failure state are test-controlled.
pub struct ControlledClock {
    now: AtomicU64,
    failing: AtomicBool,
}

impl ControlledClock {
    /// Creates a healthy clock at the supplied instant.
    #[must_use]
    pub fn new(now: UnixMillis) -> Self {
        Self {
            now: AtomicU64::new(now.get()),
            failing: AtomicBool::new(false),
        }
    }

    /// Replaces the current deterministic instant.
    pub fn set(&self, now: UnixMillis) {
        self.now.store(now.get(), Ordering::SeqCst);
    }

    /// Enables or disables the closed provider failure.
    pub fn set_failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }
}

impl Clock for ControlledClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        if self.failing.load(Ordering::SeqCst) {
            Err(ClockError::timestamp_overflow())
        } else {
            Ok(UnixMillis::new(self.now.load(Ordering::SeqCst)))
        }
    }
}

impl fmt::Debug for ControlledClock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlledClock")
            .field("failing", &self.failing.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

/// Deterministic server identity generator with optional closed failure.
pub struct ControlledInstanceIds {
    next: AtomicU64,
    failing: AtomicBool,
    trace: HarnessTrace,
}

impl ControlledInstanceIds {
    fn new(trace: HarnessTrace) -> Self {
        Self {
            next: AtomicU64::new(1),
            failing: AtomicBool::new(false),
            trace,
        }
    }

    /// Selects the next deterministic non-zero identity counter.
    pub fn set_next(&self, next: u64) {
        self.next.store(next.max(1), Ordering::SeqCst);
    }

    /// Enables or disables the closed provider failure.
    pub fn set_failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }
}

impl InstanceIdGenerator for ControlledInstanceIds {
    fn generate(&self) -> Result<InstanceId, RandomError> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(RandomError::generation_failed());
        }
        let value = self.next.fetch_add(1, Ordering::SeqCst).max(1);
        let mut bytes = [0_u8; 16];
        bytes[8..].copy_from_slice(&value.to_be_bytes());
        let identity =
            InstanceId::from_bytes(&bytes).map_err(|_| RandomError::generation_failed())?;
        self.trace.record(HarnessTraceEvent::InstanceGenerated);
        Ok(identity)
    }
}

/// Mutable current-authorization control used through the production capability.
pub struct ControlledAuthorization {
    decision: Mutex<AuthorizationDecision>,
    failing: AtomicBool,
    trace: HarnessTrace,
}

impl ControlledAuthorization {
    fn new(trace: HarnessTrace) -> Self {
        Self {
            decision: Mutex::new(AuthorizationDecision::Allow),
            failing: AtomicBool::new(false),
            trace,
        }
    }

    /// Replaces the current authorization decision.
    pub fn set_decision(&self, decision: AuthorizationDecision) {
        *lock(&self.decision) = decision;
    }

    /// Returns the current closed authorization decision.
    #[must_use]
    pub fn decision(&self) -> AuthorizationDecision {
        *lock(&self.decision)
    }

    /// Enables or disables the closed authorization-provider failure.
    pub fn set_failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }
}

impl ActionAuthorizationPort for ControlledAuthorization {
    fn authorize<'a>(
        &'a self,
        _request: ActionAuthorizationRequest<'a>,
    ) -> ActionFuture<'a, Result<AuthorizationDecision, ActionError>> {
        Box::pin(async move {
            self.trace.record(HarnessTraceEvent::Authorization);
            if self.failing.load(Ordering::SeqCst) {
                Err(ActionError::dispatcher_contract())
            } else {
                Ok(self.decision())
            }
        })
    }
}

/// Mutable application-validation control returning only typed issues.
pub struct ControlledValidation {
    issues: Mutex<Vec<ValidationIssue>>,
    failing: AtomicBool,
    trace: HarnessTrace,
}

impl ControlledValidation {
    fn new(trace: HarnessTrace) -> Self {
        Self {
            issues: Mutex::new(Vec::new()),
            failing: AtomicBool::new(false),
            trace,
        }
    }

    /// Replaces the complete deterministic issue set.
    pub fn set_issues(&self, issues: Vec<ValidationIssue>) {
        *lock(&self.issues) = issues;
    }

    /// Enables or disables the closed validation-provider failure.
    pub fn set_failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }
}

impl ValidationPort for ControlledValidation {
    fn validate<'a>(
        &'a self,
        _request: ValidationRequest<'a>,
    ) -> ValidationFuture<'a, Result<Vec<ValidationIssue>, ValidationPortError>> {
        Box::pin(async move {
            self.trace.record(HarnessTraceEvent::Validation);
            if self.failing.load(Ordering::SeqCst) {
                Err(ValidationPortError::unavailable())
            } else {
                Ok(lock(&self.issues).clone())
            }
        })
    }
}

/// One deterministic transaction-provider fault injection point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionFault {
    /// The provider remains healthy.
    None,
    /// Beginning the transaction fails.
    Begin,
    /// Committing the transaction fails.
    Commit,
    /// Rolling the transaction back fails.
    Rollback,
}

/// Deterministic host-transaction control with typed counters and faults.
#[derive(Clone)]
pub struct ControlledTransactions {
    inner: Arc<TransactionState>,
}

struct TransactionState {
    fault: AtomicU8,
    begun: AtomicUsize,
    committed: AtomicUsize,
    rolled_back: AtomicUsize,
    trace: HarnessTrace,
}

impl ControlledTransactions {
    fn new(trace: HarnessTrace) -> Self {
        Self {
            inner: Arc::new(TransactionState {
                fault: AtomicU8::new(TransactionFault::None as u8),
                begun: AtomicUsize::new(0),
                committed: AtomicUsize::new(0),
                rolled_back: AtomicUsize::new(0),
                trace,
            }),
        }
    }

    /// Replaces the active deterministic fault point.
    pub fn set_fault(&self, fault: TransactionFault) {
        self.inner.fault.store(fault as u8, Ordering::SeqCst);
    }

    /// Returns the active deterministic fault point.
    #[must_use]
    pub fn fault(&self) -> TransactionFault {
        match self.inner.fault.load(Ordering::SeqCst) {
            1 => TransactionFault::Begin,
            2 => TransactionFault::Commit,
            3 => TransactionFault::Rollback,
            _ => TransactionFault::None,
        }
    }

    /// Returns successful begin attempts.
    #[must_use]
    pub fn begun(&self) -> usize {
        self.inner.begun.load(Ordering::SeqCst)
    }

    /// Returns successful commits.
    #[must_use]
    pub fn committed(&self) -> usize {
        self.inner.committed.load(Ordering::SeqCst)
    }

    /// Returns rollback attempts.
    #[must_use]
    pub fn rolled_back(&self) -> usize {
        self.inner.rolled_back.load(Ordering::SeqCst)
    }
}

struct ControlledTransaction {
    control: ControlledTransactions,
}

impl HostTransaction for ControlledTransaction {
    fn commit(self: Box<Self>) -> LiveFuture<'static, Result<(), HostError>> {
        Box::pin(async move {
            self.control
                .inner
                .trace
                .record(HarnessTraceEvent::TransactionCommit);
            if self.control.fault() == TransactionFault::Commit {
                Err(HostError::new(HostErrorKind::Commit))
            } else {
                self.control.inner.committed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
    }

    fn rollback(self: Box<Self>) -> LiveFuture<'static, Result<(), HostError>> {
        Box::pin(async move {
            self.control
                .inner
                .trace
                .record(HarnessTraceEvent::TransactionRollback);
            self.control
                .inner
                .rolled_back
                .fetch_add(1, Ordering::SeqCst);
            if self.control.fault() == TransactionFault::Rollback {
                Err(HostError::new(HostErrorKind::Rollback))
            } else {
                Ok(())
            }
        })
    }
}

impl TransactionPort for ControlledTransactions {
    fn begin(&self) -> LiveFuture<'_, Result<Box<dyn HostTransaction>, HostError>> {
        let control = self.clone();
        Box::pin(async move {
            control
                .inner
                .trace
                .record(HarnessTraceEvent::TransactionBegin);
            if control.fault() == TransactionFault::Begin {
                return Err(HostError::new(HostErrorKind::Begin));
            }
            control.inner.begun.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(ControlledTransaction { control }) as Box<dyn HostTransaction>)
        })
    }
}

/// In-memory typed session boundary with deterministic provider failure.
pub struct ControlledSession {
    values: Mutex<BTreeMap<ModelField, SessionValue>>,
    failing: AtomicBool,
    trace: HarnessTrace,
}

impl ControlledSession {
    fn new(trace: HarnessTrace) -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
            failing: AtomicBool::new(false),
            trace,
        }
    }

    /// Seeds one already-typed registered session value.
    pub fn insert(&self, field: ModelField, value: SessionValue) {
        lock(&self.values).insert(field, value);
    }

    /// Enables or disables the closed session-provider failure.
    pub fn set_failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl SessionPort for ControlledSession {
    async fn read(&self, field: &SessionField) -> Result<Option<SessionValue>, SessionError> {
        self.trace.record(HarnessTraceEvent::SessionRead);
        if self.failing.load(Ordering::SeqCst) {
            return Err(SessionError::host_failure());
        }
        Ok(lock(&self.values).get(field.name()).cloned())
    }

    async fn apply(&self, intent: &SessionIntent) -> Result<(), SessionError> {
        self.trace.record(HarnessTraceEvent::SessionApply);
        if self.failing.load(Ordering::SeqCst) {
            return Err(SessionError::host_failure());
        }
        let mut values = lock(&self.values);
        match intent.kind() {
            SessionIntentKind::Set => {
                if let Some(value) = intent.value() {
                    values.insert(intent.field().name().clone(), value.clone());
                }
            }
            SessionIntentKind::Remove => {
                values.remove(intent.field().name());
            }
        }
        Ok(())
    }
}

/// Complete cloneable set of deterministic host dependencies for one harness.
#[derive(Clone)]
pub struct HarnessServices {
    trace: HarnessTrace,
    clock: Arc<ControlledClock>,
    instance_ids: Arc<ControlledInstanceIds>,
    authorization: Arc<ControlledAuthorization>,
    upload_authorization: Arc<ControlledUploadAuthorization>,
    validation: Arc<ControlledValidation>,
    transactions: Arc<ControlledTransactions>,
    session: Arc<ControlledSession>,
}

impl HarnessServices {
    /// Creates healthy deterministic controls at one explicit instant.
    #[must_use]
    pub fn new(now: UnixMillis) -> Self {
        let trace = HarnessTrace::default();
        Self {
            clock: Arc::new(ControlledClock::new(now)),
            instance_ids: Arc::new(ControlledInstanceIds::new(trace.clone())),
            authorization: Arc::new(ControlledAuthorization::new(trace.clone())),
            upload_authorization: Arc::new(ControlledUploadAuthorization::new()),
            validation: Arc::new(ControlledValidation::new(trace.clone())),
            transactions: Arc::new(ControlledTransactions::new(trace.clone())),
            session: Arc::new(ControlledSession::new(trace.clone())),
            trace,
        }
    }

    /// Returns the shared typed trace.
    #[must_use]
    pub const fn trace(&self) -> &HarnessTrace {
        &self.trace
    }

    /// Returns the controlled wall clock.
    #[must_use]
    pub const fn clock(&self) -> &Arc<ControlledClock> {
        &self.clock
    }

    /// Returns the deterministic server identity generator.
    #[must_use]
    pub const fn instance_ids(&self) -> &Arc<ControlledInstanceIds> {
        &self.instance_ids
    }

    /// Returns the mutable current-authorization control.
    #[must_use]
    pub const fn authorization(&self) -> &Arc<ControlledAuthorization> {
        &self.authorization
    }

    /// Returns the mutable current upload-authorization control.
    #[must_use]
    pub const fn upload_authorization(&self) -> &Arc<ControlledUploadAuthorization> {
        &self.upload_authorization
    }

    /// Returns the mutable validation control.
    #[must_use]
    pub const fn validation(&self) -> &Arc<ControlledValidation> {
        &self.validation
    }

    /// Returns the mutable transaction control.
    #[must_use]
    pub const fn transactions(&self) -> &Arc<ControlledTransactions> {
        &self.transactions
    }

    /// Returns the typed in-memory session control.
    #[must_use]
    pub const fn session(&self) -> &Arc<ControlledSession> {
        &self.session
    }
}

impl fmt::Debug for HarnessServices {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HarnessServices")
            .field("trace_events", &self.trace.events().len())
            .field("authorization", &self.authorization.decision())
            .field("transaction_fault", &self.transactions.fault())
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
