//! The event the gate dispatches for every authorization check.

use crate::events::Event;

/// Fires once for every check the gate evaluates, after the before-hooks,
/// the gate and the after-hooks have run. Mirrors Laravel's
/// `Illuminate\Auth\Access\Events\GateEvaluated`, which `Gate::raw`
/// dispatches on each check.
///
/// Every check reaches it: [`Gate::inspect`](crate::Gate::inspect),
/// [`Gate::raw`](crate::Gate::raw), their async siblings, and every form
/// built on them (`allows`, `denies`, `authorize`, `any`, `none`, `check`,
/// [`Authorizable`](crate::Authorizable), `#[authorize]`,
/// [`Gate::inspect_current`](crate::Gate::inspect_current) and
/// [`Gate::none_current`](crate::Gate::none_current)). A check of several
/// actions fires one event per action it evaluates. Use it to audit
/// authorization decisions, or to assert on them through `EventFacade::fake`.
///
/// Laravel's event holds the user object and the arguments. A gate here is
/// generic over the user and the resource, so the event names their types
/// instead: listeners stay one type for every gate, and no user or resource
/// has to be cloned into the event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateEvaluated {
    /// The `std::any::type_name` of the user the gate was asked about. A
    /// check that resolved the user itself reads it from
    /// [`Authenticatable::auth_type_name`](crate::Authenticatable::auth_type_name),
    /// so the user's own type is named even when it registered no gate.
    /// `None` when the check had no user: a guest check through
    /// [`Gate::inspect_current`](crate::Gate::inspect_current) or
    /// `#[authorize]`.
    pub user_type: Option<&'static str>,
    /// The user's identifier, `Authenticatable::get_auth_identifier`, when
    /// the check resolved the user itself: `inspect_current`, `none_current`
    /// and `#[authorize]`. `None` for a check given its user, because a
    /// generic gate user has no identifier to read, and for a guest.
    pub user_id: Option<String>,
    /// The action the check evaluated.
    pub action: String,
    /// The `std::any::type_name` of the resource the check was about.
    pub resource_type: &'static str,
    /// The decision: `Some(true)` allowed, `Some(false)` denied, `None` when
    /// nothing decided (no before-hook, no gate and no after-hook answered),
    /// which the check then answers with the default denial.
    pub decision: Option<bool>,
}

impl Event for GateEvaluated {
    fn event_name() -> &'static str {
        "Auth\\Access\\GateEvaluated"
    }
}
