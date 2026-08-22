//! Hard checker resource ceilings.

use std::error::Error;
use std::fmt;

const MAX_SOURCE_BYTES: usize = 512 * 1024;
const MAX_TEMPLATE_NODES: usize = 32_768;
const MAX_INCLUDE_DEPTH: usize = 64;
const MAX_BRANCH_STATES: usize = 256;
const MAX_HTML_TOKENS: usize = 65_536;
const MAX_ATTRIBUTES: usize = 16_384;
const MAX_STACK_DEPTH: usize = 512;
const MAX_DIAGNOSTICS: usize = 1_024;

/// Invalid checker ceiling configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckerConfigError;

impl fmt::Display for CheckerConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid_checker_limits")
    }
}

impl Error for CheckerConfigError {}

/// Complete hard resource policy for one checker run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckerLimits {
    max_source_bytes: usize,
    max_template_nodes: usize,
    max_include_depth: usize,
    max_branch_states: usize,
    max_html_tokens: usize,
    max_attributes: usize,
    max_stack_depth: usize,
    max_diagnostics: usize,
}

impl CheckerLimits {
    /// Creates nonzero ceilings within the engine's compile-time maxima.
    #[allow(
        clippy::too_many_arguments,
        reason = "the eight named resource dimensions are the public checker contract"
    )]
    pub fn new(
        max_source_bytes: usize,
        max_template_nodes: usize,
        max_include_depth: usize,
        max_branch_states: usize,
        max_html_tokens: usize,
        max_attributes: usize,
        max_stack_depth: usize,
        max_diagnostics: usize,
    ) -> Result<Self, CheckerConfigError> {
        let values = [
            (max_source_bytes, MAX_SOURCE_BYTES),
            (max_template_nodes, MAX_TEMPLATE_NODES),
            (max_include_depth, MAX_INCLUDE_DEPTH),
            (max_branch_states, MAX_BRANCH_STATES),
            (max_html_tokens, MAX_HTML_TOKENS),
            (max_attributes, MAX_ATTRIBUTES),
            (max_stack_depth, MAX_STACK_DEPTH),
            (max_diagnostics, MAX_DIAGNOSTICS),
        ];
        if values
            .into_iter()
            .any(|(value, maximum)| value == 0 || value > maximum)
        {
            return Err(CheckerConfigError);
        }
        Ok(Self {
            max_source_bytes,
            max_template_nodes,
            max_include_depth,
            max_branch_states,
            max_html_tokens,
            max_attributes,
            max_stack_depth,
            max_diagnostics,
        })
    }

    pub(crate) const fn max_source_bytes(self) -> usize {
        self.max_source_bytes
    }

    pub(crate) const fn max_template_nodes(self) -> usize {
        self.max_template_nodes
    }

    pub(crate) const fn max_include_depth(self) -> usize {
        self.max_include_depth
    }

    pub(crate) const fn max_branch_states(self) -> usize {
        self.max_branch_states
    }

    pub(crate) const fn max_html_tokens(self) -> usize {
        self.max_html_tokens
    }

    pub(crate) const fn max_attributes(self) -> usize {
        self.max_attributes
    }

    pub(crate) const fn max_stack_depth(self) -> usize {
        self.max_stack_depth
    }

    pub(crate) const fn max_diagnostics(self) -> usize {
        self.max_diagnostics
    }
}

impl Default for CheckerLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 256 * 1024,
            max_template_nodes: 8_192,
            max_include_depth: 16,
            max_branch_states: 128,
            max_html_tokens: 32_768,
            max_attributes: 2_048,
            max_stack_depth: 256,
            max_diagnostics: 64,
        }
    }
}
