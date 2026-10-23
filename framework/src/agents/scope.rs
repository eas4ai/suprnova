//! The scopes an application declares for its tools (AGENT-002).
//!
//! A scope names what a token may do, with a description the approval page
//! shows the user. The application declares its scopes once with
//! [`tool_scopes!`](crate::tool_scopes), which makes a constant of each, and
//! a `#[tool(scopes = [...])]` names those constants: a scope nobody
//! declared is an unresolved name, and the build fails.

/// One scope a token can hold: its name and the description the approval
/// page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scope {
    name: &'static str,
    description: &'static str,
}

impl Scope {
    /// The scope `name`, shown to the user as `description`. Called by
    /// `tool_scopes!`, which also checks both at build time.
    #[doc(hidden)]
    pub const fn __declare(name: &'static str, description: &'static str) -> Self {
        Self { name, description }
    }

    /// The scope's name, as a token carries it.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The description the approval page shows.
    pub const fn description(&self) -> &'static str {
        self.description
    }

    /// Whether `name` is a scope name OAuth allows (RFC 6749, section 3.3):
    /// one or more visible ASCII characters other than space, `"` and `\`.
    #[doc(hidden)]
    pub const fn __is_valid_name(name: &str) -> bool {
        let bytes = name.as_bytes();
        if bytes.is_empty() {
            return false;
        }
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if !(byte == 0x21 || (byte >= 0x23 && byte <= 0x5B) || (byte >= 0x5D && byte <= 0x7E)) {
                return false;
            }
            index += 1;
        }
        true
    }

    /// Whether `description` has a character other than whitespace.
    #[doc(hidden)]
    pub const fn __is_valid_description(description: &str) -> bool {
        let bytes = description.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if !bytes[index].is_ascii_whitespace() {
                return true;
            }
            index += 1;
        }
        false
    }
}

impl AsRef<str> for Scope {
    fn as_ref(&self) -> &str {
        self.name
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

/// The record `tool_scopes!` submits for each scope it declares, so the
/// application knows every scope it may grant.
pub struct ScopeRecord {
    scope: Scope,
}

impl ScopeRecord {
    /// The record of `scope`. Built by `tool_scopes!`.
    #[doc(hidden)]
    pub const fn __new(scope: Scope) -> Self {
        Self { scope }
    }

    /// The declared scope.
    pub fn scope(&self) -> Scope {
        self.scope
    }
}

inventory::collect!(ScopeRecord);

/// Every scope the application declared, sorted by name, each once. Two
/// declarations of one name are one scope when their descriptions agree;
/// [`ToolRegistry::build`](super::ToolRegistry::build) refuses two that
/// disagree.
pub fn declared_scopes() -> Vec<Scope> {
    let mut scopes: Vec<Scope> = inventory::iter::<ScopeRecord>()
        .map(ScopeRecord::scope)
        .collect();
    scopes.sort_by(|a, b| (a.name, a.description).cmp(&(b.name, b.description)));
    scopes.dedup();
    scopes
}

/// Declare the scopes an application's tools need, each with the
/// description the approval page shows the user (AGENT-002).
///
/// Each line makes a constant of type [`Scope`](crate::agents::Scope) in the
/// module, which `#[tool(scopes = [...])]` names. A tool naming a scope no
/// `tool_scopes!` declared does not build, and a scope name OAuth does not
/// allow (empty, or with a space, `"` or `\`) or an empty description does
/// not build either.
///
/// ```rust
/// suprnova::tool_scopes! {
///     pub mod scope {
///         POSTS_READ = "posts:read" => "Read your posts";
///         POSTS_WRITE = "posts:write" => "Create and edit your posts";
///     }
/// }
///
/// assert_eq!(scope::POSTS_READ.name(), "posts:read");
/// assert_eq!(scope::POSTS_WRITE.description(), "Create and edit your posts");
/// ```
#[macro_export]
macro_rules! tool_scopes {
    (
        $(#[$meta:meta])*
        $vis:vis mod $module:ident {
            $( $(#[$item_meta:meta])* $name:ident = $scope:literal => $description:literal; )*
        }
    ) => {
        $(#[$meta])*
        $vis mod $module {
            $(
                $(#[$item_meta])*
                #[doc = $description]
                pub const $name: $crate::agents::Scope =
                    $crate::agents::Scope::__declare($scope, $description);
                const _: () = ::core::assert!(
                    $crate::agents::Scope::__is_valid_name($scope),
                    ::core::concat!(
                        "invalid scope name `", $scope, "`: a scope name is one or more visible ",
                        "ASCII characters other than space, `\"` and `\\`"
                    )
                );
                const _: () = ::core::assert!(
                    $crate::agents::Scope::__is_valid_description($description),
                    ::core::concat!(
                        "the scope `", $scope, "` needs a description the approval page can show"
                    )
                );
                $crate::inventory::submit! {
                    $crate::agents::ScopeRecord::__new($name)
                }
            )*
        }
    };
}
