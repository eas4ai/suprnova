//! Topic invalidations share Live authority and carry no model values.

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use super::async_updates::{AsyncState, StreamPayloadSpec};
use super::{LiveRegistry, LiveRuntime};
use crate::broadcasting::{
    BroadcastHub, InMemoryBroadcastHub, LiveTopicExclusion, LiveTopicSender, LiveTopicSignal,
};
use crate::{App, FrameworkError};
use sha2::{Digest, Sha256};
use suprnova_live::async_updates::{
    BoundedEventNames, BoundedTopics, ReconnectPolicy, StreamName, SubscriptionMetadata,
    SubscriptionMode, SubscriptionModes, TopicName,
};
use suprnova_live::identity::{ComponentName, ViewName};
use suprnova_live::metadata::{ComponentMetadata, ContractVersions};
use suprnova_live::registry::ComponentDescriptor;

pub(crate) fn topic_component(topic: &str) -> String {
    format!(
        "suprnova.topic.{}",
        hex::encode(Sha256::digest(topic.as_bytes()))
    )
}

pub(crate) fn topic_descriptor(
    topic: &str,
    family: bool,
) -> Result<ComponentDescriptor, FrameworkError> {
    let invalid = || FrameworkError::internal(format!("invalid Live topic `{topic}`"));
    let name = topic_component(topic);
    let pattern = if family {
        format!("{topic}.:topic_key")
    } else {
        topic.to_owned()
    };
    let subscription = SubscriptionMetadata::new(
        StreamName::parse("changes").map_err(|_| invalid())?,
        BoundedTopics::new(vec![TopicName::parse(&pattern).map_err(|_| invalid())?])
            .map_err(|_| invalid())?,
        BoundedEventNames::refresh_only(),
        SubscriptionModes::new(vec![
            SubscriptionMode::ServerSentEvents,
            SubscriptionMode::WebSocket,
        ])
        .map_err(|_| invalid())?,
        ReconnectPolicy::ResumeOrRefresh {
            maximum_attempts: std::num::NonZeroU8::new(4).ok_or_else(invalid)?,
        },
    );
    let metadata = ComponentMetadata::new_with_async_contracts(
        ComponentName::parse(&name).map_err(|_| invalid())?,
        ViewName::parse(&format!("live/topics/{name}.html")).map_err(|_| invalid())?,
        ContractVersions::new(2, 1, 1, 1, 1).map_err(|_| invalid())?,
        vec![],
        vec![],
        vec![],
        vec![],
        vec![subscription],
        false,
    )
    .map_err(|_| invalid())?;
    Ok(ComponentDescriptor::new(metadata))
}

type ModelBinding =
    Arc<dyn Fn(&dyn Any) -> Result<(String, Vec<String>), FrameworkError> + Send + Sync>;

pub(crate) struct TopicState {
    pub(crate) registry: LiveRegistry,
    pub(crate) hub: Arc<dyn BroadcastHub>,
    interval: Duration,
    listeners: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    models: Mutex<HashMap<TypeId, Vec<ModelBinding>>>,
}

impl TopicState {
    pub(crate) fn new(registry: LiveRegistry, interval: Duration) -> Self {
        let hub = App::make::<dyn BroadcastHub>()
            .unwrap_or_else(|| Arc::new(InMemoryBroadcastHub::new()));
        Self {
            registry,
            hub,
            interval,
            listeners: Mutex::new(HashMap::new()),
            models: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn listen(
        &self,
        state: &Arc<AsyncState>,
        topic: &str,
    ) -> Result<(), FrameworkError> {
        let mut listeners = self
            .listeners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if listeners.contains_key(topic) {
            return Ok(());
        }
        let mut receiver = self.hub.subscribe_live_topic(topic)?;
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            FrameworkError::internal("Live topic subscription requires an async runtime")
        })?;
        let weak: Weak<AsyncState> = Arc::downgrade(state);
        let topic_name = topic.to_owned();
        let task = handle.spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(envelope) => {
                        let Some(state) = weak.upgrade() else {
                            break;
                        };
                        let signal: LiveTopicSignal = match serde_json::from_value(envelope.data) {
                            Ok(signal) => signal,
                            Err(error) => {
                                tracing::error!(%error, "invalid Live topic hub signal");
                                continue;
                            }
                        };
                        if signal.topic != topic_name {
                            continue;
                        }
                        let spec = StreamPayloadSpec::TopicChange {
                            signal,
                            except: envelope.except,
                        };
                        if let Err(error) = state.publish(&topic_name, &spec).await {
                            tracing::error!(?error, "Live topic delivery failed");
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let Some(state) = weak.upgrade() else {
                            break;
                        };
                        // A fresh render restores props when the hub's bounded feed loses a tail.
                        if let Err(error) = state
                            .publish(&topic_name, &StreamPayloadSpec::Refresh)
                            .await
                        {
                            tracing::error!(?error, "Live topic recovery failed");
                        }
                    }
                }
            }
        });
        listeners.insert(topic.to_owned(), task);
        Ok(())
    }

    pub(crate) fn start(&self, state: &Arc<AsyncState>) -> Result<(), FrameworkError> {
        for topic in self.registry.topics().keys() {
            self.listen(state, topic)?;
        }
        Ok(())
    }
}

impl Drop for TopicState {
    fn drop(&mut self) {
        for task in self
            .listeners
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
        {
            task.abort();
        }
    }
}

/// Publishes data-free changes through the bound Live runtime's topic layer.
#[derive(Clone)]
pub struct LiveTopics {
    state: Arc<AsyncState>,
}

impl LiveTopics {
    /// Resolves the runtime used by island and page subscriptions.
    pub fn resolve() -> Result<Self, FrameworkError> {
        // Model bindings are installed during bootstrap, before Server prepares Crypt.
        if !crate::Crypt::is_initialized() {
            let environment = crate::Config::get::<crate::config::AppConfig>()
                .map(|config| config.environment)
                .unwrap_or_else(crate::config::Environment::detect);
            crate::crypto::initialize_from_environment(&environment)?;
        }
        Ok(Self::from_runtime(&LiveRuntime::bind()?))
    }

    /// Uses an explicit runtime, including independent runtime instances in tests.
    #[must_use]
    pub fn from_runtime(runtime: &LiveRuntime) -> Self {
        Self {
            state: Arc::clone(runtime.async_state()),
        }
    }

    /// Reads the hub's cursor, or the process-local cursor without a configured hub.
    pub async fn cursor(&self, topic: &str) -> Result<u64, FrameworkError> {
        self.check_topic(topic)?;
        self.state.topics.listen(&self.state, topic)?;
        self.state.topics.hub.live_topic_cursor(topic).await
    }

    fn check_topic(&self, topic: &str) -> Result<(), FrameworkError> {
        TopicName::parse(topic).map_err(|_| FrameworkError::internal("invalid Live topic"))?;
        if self
            .state
            .topics
            .registry
            .application_topic(topic)
            .is_some()
        {
            return Ok(());
        }
        if let Some((component, stream)) = topic.split_once("::") {
            if let Ok(component) = ComponentName::parse(component) {
                if self
                    .state
                    .topics
                    .registry
                    .topic_for_component(&component)
                    .is_none()
                    && let Ok(descriptor) = self.state.topics.registry.engine().resolve(&component)
                {
                    if descriptor
                        .metadata()
                        .subscriptions()
                        .iter()
                        .any(|s| s.stream().as_str() == stream)
                    {
                        return Ok(());
                    }
                }
            }
        }
        Err(FrameworkError::internal(format!(
            "Live topic `{topic}` is not registered"
        )))
    }

    /// Defers a change until its write commits and merges it into the current unit of work.
    pub async fn changed(&self, topic: &str, props: &[&str]) -> Result<(), FrameworkError> {
        let (pending, buffer) = self.prepare_change(topic, props).await?;
        crate::DB::after_commit(move || pending.enqueue(buffer)).await
    }

    /// Defers a change on an explicit transaction handle, which is not ambient state.
    pub async fn changed_with_tx(
        &self,
        transaction: &crate::Transaction,
        topic: &str,
        props: &[&str],
    ) -> Result<(), FrameworkError> {
        let (pending, buffer) = self.prepare_change(topic, props).await?;
        transaction.after_commit(move || pending.enqueue(buffer));
        Ok(())
    }

    async fn prepare_change(
        &self,
        topic: &str,
        props: &[&str],
    ) -> Result<(Pending, Option<Registration>), FrameworkError> {
        self.check_topic(topic)?;
        suprnova_live::async_updates::RegisteredTopicChange::new(
            topic.to_owned(),
            1,
            props.iter().map(|p| (*p).to_owned()).collect(),
        )
        .map_err(|_| FrameworkError::internal("invalid Live topic prop names"))?;
        let except = if let Some(socket) = crate::broadcasting::request_socket::current() {
            let sender = if let Some(session) = crate::session::session()
                .filter(|session| crate::session::is_valid_session_id(&session.id))
            {
                Some(LiveTopicSender::Session(session.id))
            } else {
                crate::auth::Auth::route_principal()
                    .await?
                    .map(LiveTopicSender::Principal)
            };
            sender.map(|sender| LiveTopicExclusion { socket, sender })
        } else {
            None
        };
        let pending = Pending {
            publisher: self.clone(),
            topic: topic.to_owned(),
            props: (!props.is_empty()).then(|| props.iter().map(|p| (*p).to_owned()).collect()),
            except,
        };
        let buffer = CHANGES
            .try_with(|buffer| Registration::new(Arc::clone(buffer)))
            .ok();
        Ok((pending, buffer))
    }

    /// Coalesces changes in an explicit request-like or job-like scope.
    /// Nested batches reuse the enclosing unit so they cannot flush it early.
    pub async fn batch<F: Future>(&self, future: F) -> Result<F::Output, FrameworkError> {
        if CHANGES.try_with(|_| ()).is_ok() {
            Ok(future.await)
        } else {
            scope(future).await
        }
    }

    /// Keeps page-bindable refreshes on the shared publication authority.
    pub(crate) async fn refresh_shared(
        state: &Arc<AsyncState>,
        topic: &str,
    ) -> Option<Result<(), FrameworkError>> {
        let topics = Self {
            state: Arc::clone(state),
        };
        if topics.check_topic(topic).is_err() {
            return None;
        }
        Some(topics.changed(topic, &[]).await)
    }

    /// Merges replay changes after their transactions resolve, including late manual commits.
    pub async fn suppress<F, Fut>(&self, replay: F) -> Result<Fut::Output, FrameworkError>
    where
        F: FnOnce() -> Fut,
        Fut: Future,
    {
        let outer = CHANGES
            .try_with(|buffer| Registration::new(Arc::clone(buffer)))
            .ok();
        let buffer = Arc::new(Mutex::new(ChangeBuffer {
            forward: outer,
            ..ChangeBuffer::default()
        }));
        let mut guard = BufferGuard(Some(Arc::clone(&buffer)));
        let output = CHANGES.scope(Arc::clone(&buffer), replay()).await;
        guard.0.take();
        flush(close(&buffer)).await?;
        Ok(output)
    }

    /// Registers saved and deleted notifications for a model without serializing its values.
    pub fn on_model<M: 'static>(&self, topic: &str, props: &[&str]) -> Result<(), FrameworkError> {
        self.check_topic(topic)?;
        suprnova_live::async_updates::RegisteredTopicChange::new(
            topic.to_owned(),
            1,
            props.iter().map(|prop| (*prop).to_owned()).collect(),
        )
        .map_err(|_| FrameworkError::internal("invalid Live model topic prop names"))?;
        let mut models = self
            .state
            .topics
            .models
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let topic = topic.to_owned();
        let props: Vec<String> = props.iter().map(|p| (*p).to_owned()).collect();
        models
            .entry(TypeId::of::<M>())
            .or_default()
            .push(Arc::new(move |_| Ok((topic.clone(), props.clone()))));
        Ok(())
    }

    /// Derives the private instance from the saved or deleted model, without sending its values.
    pub fn on_model_scoped<M: 'static>(
        &self,
        family: &str,
        key: impl Fn(&M) -> String + Send + Sync + 'static,
        props: &[&str],
    ) -> Result<(), FrameworkError> {
        let registry = self.state.topics.registry.clone();
        let sample = registry.family_instance(family, "0")?;
        suprnova_live::async_updates::RegisteredTopicChange::new(
            sample,
            1,
            props.iter().map(|p| (*p).to_owned()).collect(),
        )
        .map_err(|_| FrameworkError::internal("invalid Live model topic prop names"))?;
        let family = family.to_owned();
        let props: Vec<String> = props.iter().map(|p| (*p).to_owned()).collect();
        self.state
            .topics
            .models
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(TypeId::of::<M>())
            .or_default()
            .push(Arc::new(move |model| {
                let model = model
                    .downcast_ref::<M>()
                    .ok_or_else(|| FrameworkError::internal("Live scoped model type mismatch"))?;
                Ok((
                    registry.family_instance(&family, &key(model))?,
                    props.clone(),
                ))
            }));
        Ok(())
    }

    /// Called by generated saved and deleted hooks after the write succeeds.
    #[doc(hidden)]
    pub async fn __model_changed<M: 'static>(model: &M) -> Result<(), FrameworkError> {
        let Some((publisher, bindings)) = Self::model_bindings::<M>() else {
            return Ok(());
        };
        for binding in bindings {
            let (topic, props) = binding(model)?;
            publisher
                .changed(
                    &topic,
                    &props.iter().map(String::as_str).collect::<Vec<_>>(),
                )
                .await?;
        }
        Ok(())
    }

    /// Called by generated hooks when a model write names a transaction handle.
    #[doc(hidden)]
    pub async fn __model_changed_with_tx<M: 'static>(
        model: &M,
        transaction: &crate::Transaction,
    ) -> Result<(), FrameworkError> {
        let Some((publisher, bindings)) = Self::model_bindings::<M>() else {
            return Ok(());
        };
        for binding in bindings {
            let (topic, props) = binding(model)?;
            publisher
                .changed_with_tx(
                    transaction,
                    &topic,
                    &props.iter().map(String::as_str).collect::<Vec<_>>(),
                )
                .await?;
        }
        Ok(())
    }

    fn model_bindings<M: 'static>() -> Option<(Self, Vec<ModelBinding>)> {
        if crate::seed::events_muted() {
            return None;
        }
        let runtime = App::resolve::<LiveRuntime>().ok()?;
        let publisher = Self::from_runtime(&runtime);
        let bindings = publisher
            .state
            .topics
            .models
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&TypeId::of::<M>())
            .cloned()
            .unwrap_or_default();
        Some((publisher, bindings))
    }
}

struct Pending {
    publisher: LiveTopics,
    topic: String,
    props: Option<BTreeSet<String>>,
    except: Option<LiveTopicExclusion>,
}
impl Pending {
    async fn enqueue(self, buffer: Option<Registration>) -> Result<(), FrameworkError> {
        match buffer {
            Some(registration) => registration.enqueue(self).await,
            None => self.publish().await,
        }
    }

    async fn publish(self) -> Result<(), FrameworkError> {
        self.publisher
            .state
            .topics
            .listen(&self.publisher.state, &self.topic)?;
        self.publisher
            .state
            .topics
            .hub
            .publish_live_topic(
                &self.topic,
                &self
                    .props
                    .map(|props| props.into_iter().collect::<Vec<_>>())
                    .unwrap_or_default(),
                self.except,
                self.publisher.state.topics.interval,
            )
            .await
    }
}
#[derive(Default)]
struct ChangeBuffer {
    closed: bool,
    outstanding: usize,
    forward: Option<Registration>,
    entries: BTreeMap<(usize, String), Pending>,
}
type Buffer = Arc<Mutex<ChangeBuffer>>;
tokio::task_local! { static CHANGES: Buffer; }

// An after-commit callback holds its buffer open even when its transaction
// outlives the replay closure or request that registered the change.
struct Registration(Option<Buffer>);
impl Registration {
    fn new(buffer: Buffer) -> Self {
        buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .outstanding += 1;
        Self(Some(buffer))
    }

    async fn enqueue(mut self, pending: Pending) -> Result<(), FrameworkError> {
        let batch = self.0.take().and_then(|buffer| {
            let mut buffer = buffer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            merge(&mut buffer, pending);
            release(&mut buffer)
        });
        flush(batch).await
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(buffer) = self.0.take() {
            let batch = release(
                &mut buffer
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            flush_detached(batch);
        }
    }
}

fn merge(buffer: &mut ChangeBuffer, pending: Pending) {
    let key = (
        Arc::as_ptr(&pending.publisher.state) as usize,
        pending.topic.clone(),
    );
    match buffer.entries.get_mut(&key) {
        Some(existing) => {
            if let Some(current) = existing.props.as_mut() {
                match pending.props {
                    Some(incoming) => current.extend(incoming),
                    None => existing.props = None,
                }
                // Keep a large committed union deliverable within the wire budget.
                if existing
                    .props
                    .as_ref()
                    .is_some_and(|props| props.len() > 256)
                {
                    existing.props = None;
                }
            }
            if existing.except != pending.except {
                existing.except = None;
            }
        }
        None => {
            buffer.entries.insert(key, pending);
        }
    }
}

struct ReadyBatch {
    changes: Vec<Pending>,
    forward: Option<Registration>,
}
fn ready(buffer: &mut ChangeBuffer) -> Option<ReadyBatch> {
    (buffer.closed && buffer.outstanding == 0).then(|| ReadyBatch {
        changes: std::mem::take(&mut buffer.entries).into_values().collect(),
        forward: buffer.forward.take(),
    })
}
fn release(buffer: &mut ChangeBuffer) -> Option<ReadyBatch> {
    buffer.outstanding -= 1;
    ready(buffer)
}
fn close(buffer: &Buffer) -> Option<ReadyBatch> {
    let mut buffer = buffer
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    buffer.closed = true;
    ready(&mut buffer)
}
async fn flush(mut batch: Option<ReadyBatch>) -> Result<(), FrameworkError> {
    let mut failure = None;
    while let Some(ReadyBatch {
        changes,
        mut forward,
    }) = batch.take()
    {
        if let Some(buffer) = forward
            .as_mut()
            .and_then(|registration| registration.0.take())
        {
            let mut buffer = buffer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for change in changes {
                merge(&mut buffer, change);
            }
            batch = release(&mut buffer);
        } else {
            for change in changes {
                if let Err(error) = change.publish().await {
                    if failure.is_none() {
                        failure = Some(error);
                    }
                }
            }
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
fn flush_detached(batch: Option<ReadyBatch>) {
    if batch.is_some() {
        crate::database::after_commit::spawn_detached(async move {
            if let Err(error) = flush(batch).await {
                tracing::error!(%error, "committed Live topic change could not flush");
            }
        });
    }
}
struct BufferGuard(Option<Buffer>);
impl Drop for BufferGuard {
    fn drop(&mut self) {
        if let Some(buffer) = self.0.take() {
            flush_detached(close(&buffer));
        }
    }
}

pub(crate) async fn scope<F: Future>(future: F) -> Result<F::Output, FrameworkError> {
    let buffer = Arc::new(Mutex::new(ChangeBuffer::default()));
    let mut guard = BufferGuard(Some(Arc::clone(&buffer)));
    let output = CHANGES.scope(Arc::clone(&buffer), future).await;
    guard.0.take();
    flush(close(&buffer)).await?;
    Ok(output)
}

pub(crate) async fn scope_request<F>(
    future: F,
) -> hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, std::convert::Infallible>>
where
    F: Future<
        Output = hyper::Response<
            http_body_util::combinators::BoxBody<bytes::Bytes, std::convert::Infallible>,
        >,
    >,
{
    match scope(future).await {
        Ok(response) => response,
        Err(error) => crate::HttpResponse::new()
            .status(500)
            .with_reported_error_from(&error)
            .into_hyper(),
    }
}

pub(crate) fn register_subscription_mounts(runtime: &LiveRuntime) -> Result<(), FrameworkError> {
    use suprnova_live::host::{
        MountCatalogEntry, MountScopeRequirements, MountSelection, ScopeRequirement,
    };
    use suprnova_live::identity::{BuildId, IslandSlot, RouteIdentity};
    use suprnova_live::mount::DocumentMountKey;
    use suprnova_live::snapshot::{ComponentContract as SnapshotContract, ExpectedSeedV1};
    for name in runtime.topic_registry().component_names() {
        let descriptor = runtime
            .topic_registry()
            .engine()
            .resolve(&name)
            .map_err(|_| FrameworkError::internal("Live topic component is unavailable"))?;
        if descriptor.metadata().subscriptions().is_empty() {
            continue;
        }
        let build = || -> Result<_, FrameworkError> {
            let invalid = || FrameworkError::internal("Live subscription mount was rejected");
            let schemas = descriptor.snapshot_schemas().map_err(|_| invalid())?;
            let versions = descriptor.metadata().versions();
            let route_digest = Sha256::digest(
                format!("suprnova/live/topic-mount/v1:{}", name.as_str()).as_bytes(),
            );
            let route = RouteIdentity::from_bytes(&route_digest).map_err(|_| invalid())?;
            let slot = IslandSlot::parse("topic").map_err(|_| invalid())?;
            let key = DocumentMountKey::parse("topic").map_err(|_| invalid())?;
            let build = BuildId::parse(concat!("suprnova-", env!("CARGO_PKG_VERSION")))
                .map_err(|_| invalid())?;
            let contract = SnapshotContract::new(
                name.clone(),
                descriptor.contract_digest().clone(),
                schemas.state().version(),
                schemas.memo().version(),
                schemas.mount().version(),
            )
            .map_err(|_| invalid())?;
            let expected = ExpectedSeedV1::new(
                contract,
                build.clone(),
                route.clone(),
                slot.clone(),
                schemas,
            );
            let selection = MountSelection::new(
                route,
                slot,
                name.clone(),
                descriptor.contract_digest().clone(),
                versions.minimum_protocol(),
            );
            let public = runtime
                .topic_registry()
                .topic_for_component(&name)
                .is_some_and(|(_, public)| public);
            let presence = if public {
                ScopeRequirement::Optional
            } else {
                ScopeRequirement::Required
            };
            let requirements =
                MountScopeRequirements::new(presence, presence, ScopeRequirement::Optional);
            Ok(super::runtime::LiveMountRegistration::new(
                MountCatalogEntry::new(expected, requirements).with_document_key(key.clone()),
                selection,
                key,
                build,
                if public {
                    super::document::LiveMountKind::PublicSeed
                } else {
                    super::document::LiveMountKind::IdentityBound
                },
            ))
        };
        runtime.register_mount(build()?)?;
    }
    Ok(())
}

impl LiveTopics {
    pub(crate) async fn page_bindings(
        &self,
        bindings: &[crate::inertia::LiveBinding],
        page_component: &str,
        scopes: &BTreeMap<String, String>,
    ) -> Result<serde_json::Value, FrameworkError> {
        let runtime = LiveRuntime::bind()?;
        let registry = &self.state.topics.registry;
        let principal = super::ports::route_principal().await;
        let mut selected: BTreeMap<String, (Vec<String>, String, String, Option<String>)> =
            BTreeMap::new();
        for binding in bindings {
            let (topic, component, stream, key) = if registry.is_family(binding.topic) {
                let key = scopes.get(binding.topic).ok_or_else(|| {
                    FrameworkError::internal(format!(
                        "Live page `{page_component}` is missing scope for family `{}`",
                        binding.topic
                    ))
                })?;
                (
                    registry.family_instance(binding.topic, key)?,
                    topic_component(binding.topic),
                    "changes".to_owned(),
                    Some(key.clone()),
                )
            } else if registry.topics().contains_key(binding.topic) {
                (
                    binding.topic.to_owned(),
                    topic_component(binding.topic),
                    "changes".to_owned(),
                    None,
                )
            } else {
                let (component, stream) = binding.topic.split_once("::").ok_or_else(|| {
                    FrameworkError::internal(format!(
                        "Live page `{page_component}` binds unregistered topic `{}`",
                        binding.topic
                    ))
                })?;
                (
                    binding.topic.to_owned(),
                    component.to_owned(),
                    stream.to_owned(),
                    None,
                )
            };
            self.check_topic(&topic)?;
            let component_name = ComponentName::parse(&component)
                .map_err(|_| FrameworkError::internal("invalid Live binding component"))?;
            let stream_name = StreamName::parse(&stream)
                .map_err(|_| FrameworkError::internal("invalid Live binding stream"))?;
            let authority = if registry.application_topic(&topic).is_some() {
                super::ports::subscription::SubscriptionAuthority::registered(
                    registry,
                    &component_name,
                    &stream_name,
                    [topic.clone()],
                )
            } else {
                super::ports::subscription::SubscriptionAuthority::component_stream(
                    &component_name,
                    &stream_name,
                )
            };
            if !authority.allows(principal.as_deref()).await {
                omitted_binding(page_component, &topic);
                continue;
            }
            let entry = selected
                .entry(topic)
                .or_insert_with(|| (Vec::new(), component, stream, key));
            for prop in binding.only {
                if !entry.0.iter().any(|name| name == prop) {
                    entry.0.push((*prop).to_owned());
                }
            }
        }
        if selected.is_empty() {
            return Ok(serde_json::json!({}));
        }
        let names: Vec<_> = selected.keys().cloned().collect();
        let cursors = self.state.topics.hub.live_topic_cursors(&names).await?;
        if cursors.len() != names.len() {
            return Err(FrameworkError::internal(
                "Live topic baseline has an invalid cursor count",
            ));
        }
        let mut result = serde_json::Map::new();
        for ((topic, (only, component, stream, key)), cursor) in selected.into_iter().zip(cursors) {
            let island = runtime.subscription_mount(&component, key.as_deref())?;
            result.insert(topic, serde_json::json!({"only": only, "cursor": cursor, "island": island, "stream": stream}));
        }
        Ok(serde_json::Value::Object(result))
    }
}

fn omitted_binding(page_component: &str, topic: &str) {
    static WARNED: std::sync::OnceLock<Mutex<BTreeSet<(String, String)>>> =
        std::sync::OnceLock::new();
    let first = WARNED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert((page_component.to_owned(), topic.to_owned()));
    if first {
        tracing::warn!(
            page_component,
            topic,
            "Live page binding is not authorized; omitting it"
        );
    } else {
        tracing::debug!(
            page_component,
            topic,
            "Live page binding is not authorized; omitting it"
        );
    }
}
