//! Lightweight role and permission authorization.
//!
//! The RBAC module stores framework-owned roles, permissions, and
//! polymorphic model assignments. Consumer apps opt in by registering
//! [`migrations::CreateRbacTables`] in their migrator and implementing
//! [`HasRoles`] on their authenticatable user model.
//!
//! Granting and revoking are separate calls, each naming exactly one
//! assignment, and there is deliberately no bulk "remove everything from
//! this model" helper: a revocation that removes more than it names is the
//! failure mode this module refuses. Where several of them have to land
//! together - the provisioning flow that grants one role and takes another
//! away in the same step - wrap the sequence in
//! [`DB::transaction`](crate::DB::transaction). Every statement here
//! resolves its executor through the framework's transaction-first
//! resolution, so the whole sequence joins the transaction in scope and
//! commits or rolls back as one unit; a rollback puts back exactly what the
//! revocation removed.
//!
//! RBAC and the [`Gate`](crate::Gate) do not know each other until the
//! application opts in with [`register_gate_bridge`]: from then on every
//! permission a user holds also answers the gate's async forms as an ability
//! of the same name, and every other ability still goes to the gate's
//! definitions and policies.

pub mod entity;
mod gate_bridge;
mod has_roles;
mod middleware;
pub mod migrations;

pub use gate_bridge::{GateBridgeMiddleware, register_gate_bridge};
#[cfg(any(test, feature = "testing"))]
pub use has_roles::observed_rbac_statements_for_test;
pub use has_roles::{
    HasRoles, assign_role_to_model, assign_role_to_model_on_guard, create_permission,
    create_permission_on_guard, create_role, create_role_on_guard, give_permission_to_model,
    give_permission_to_model_on_guard, give_permission_to_role, give_permission_to_role_on_guard,
    has_permission_for_model, has_permission_for_model_on_guard, has_role_for_model,
    has_role_for_model_on_guard, remove_permission_from_model,
    remove_permission_from_model_on_guard, remove_permission_from_role,
    remove_permission_from_role_on_guard, remove_role_from_model, remove_role_from_model_on_guard,
};
pub use middleware::{PermissionMiddleware, RoleMiddleware};
