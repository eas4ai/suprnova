//! The `{name}` authorization policy for `{model}`.

use suprnova::policy;

use crate::models::user::User;{model_import}

/// Who may do what to a `{model}`. `#[policy]` registers each method as a
/// gate named for the method and the model, so `update` is the gate
/// `update-{model_kebab}`. An ability with no method is denied, and every
/// method here denies until you write its rule.
pub struct {name};

#[policy(User, {model})]
impl {name} {
    /// Whether the user may list `{model}` models.
    fn view_any(_user: &User, _{model_param}: &{model}) -> bool {
        false
    }

    /// Whether the user may view the `{model}`.
    fn view(_user: &User, _{model_param}: &{model}) -> bool {
        false
    }

    /// Whether the user may create a `{model}`.
    fn create(_user: &User, _{model_param}: &{model}) -> bool {
        false
    }

    /// Whether the user may update the `{model}`.
    fn update(_user: &User, _{model_param}: &{model}) -> bool {
        false
    }

    /// Whether the user may delete the `{model}`.
    fn delete(_user: &User, _{model_param}: &{model}) -> bool {
        false
    }

    /// Whether the user may restore the soft-deleted `{model}`.
    fn restore(_user: &User, _{model_param}: &{model}) -> bool {
        false
    }

    /// Whether the user may delete the `{model}` for good.
    fn force_delete(_user: &User, _{model_param}: &{model}) -> bool {
        false
    }
}
