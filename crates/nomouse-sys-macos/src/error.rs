//! Accessibility error codes (`AXError`), decoded into a Rust enum.
//!
//! This module is plain Rust so it compiles and is tested on every OS.

use std::fmt;

/// A failed Accessibility call, decoded from the `AXError` code it returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AxError {
    /// `kAXErrorFailure`: a system error occurred.
    Failure,
    /// `kAXErrorIllegalArgument`: an argument was invalid.
    IllegalArgument,
    /// `kAXErrorInvalidUIElement`: the element no longer exists.
    InvalidElement,
    /// `kAXErrorCannotComplete`: the application did not answer (busy or timed out).
    CannotComplete,
    /// `kAXErrorAttributeUnsupported`: the element has no such attribute.
    AttributeUnsupported,
    /// `kAXErrorActionUnsupported`: the element has no such action.
    ActionUnsupported,
    /// `kAXErrorNotImplemented`: the application does not implement the Accessibility API.
    NotImplemented,
    /// `kAXErrorAPIDisabled`: this process is not trusted for Accessibility.
    ApiDisabled,
    /// `kAXErrorNoValue`: the attribute exists but has no value.
    NoValue,
    /// Any other code, kept verbatim.
    Other(i32),
}

impl AxError {
    /// Decodes an `AXError` code; `None` for `kAXErrorSuccess` (0).
    pub const fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            0 => return None,
            -25200 => Self::Failure,
            -25201 => Self::IllegalArgument,
            -25202 => Self::InvalidElement,
            -25204 => Self::CannotComplete,
            -25205 => Self::AttributeUnsupported,
            -25206 => Self::ActionUnsupported,
            -25208 => Self::NotImplemented,
            -25211 => Self::ApiDisabled,
            -25212 => Self::NoValue,
            other => Self::Other(other),
        })
    }

    /// Whether the error means "this attribute is simply absent", which readers treat as no value.
    pub const fn is_missing_value(self) -> bool {
        matches!(
            self,
            Self::NoValue | Self::AttributeUnsupported | Self::NotImplemented
        )
    }
}

impl fmt::Display for AxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failure => f.write_str("accessibility call failed"),
            Self::IllegalArgument => f.write_str("illegal accessibility argument"),
            Self::InvalidElement => f.write_str("the element no longer exists"),
            Self::CannotComplete => f.write_str("the application did not respond in time"),
            Self::AttributeUnsupported => f.write_str("attribute unsupported"),
            Self::ActionUnsupported => f.write_str("action unsupported"),
            Self::NotImplemented => f.write_str("the application does not support accessibility"),
            Self::ApiDisabled => f.write_str("this process is not trusted for Accessibility"),
            Self::NoValue => f.write_str("the attribute has no value"),
            Self::Other(code) => write!(f, "accessibility error {code}"),
        }
    }
}

impl std::error::Error for AxError {}

#[cfg(test)]
mod tests {
    use super::AxError;

    #[test]
    fn success_is_not_an_error() {
        assert_eq!(AxError::from_code(0), None);
    }

    #[test]
    fn known_codes_decode() {
        assert_eq!(AxError::from_code(-25211), Some(AxError::ApiDisabled));
        assert_eq!(AxError::from_code(-25204), Some(AxError::CannotComplete));
        assert_eq!(AxError::from_code(-1), Some(AxError::Other(-1)));
    }

    #[test]
    fn missing_value_classification() {
        assert!(AxError::NoValue.is_missing_value());
        assert!(AxError::AttributeUnsupported.is_missing_value());
        assert!(!AxError::ApiDisabled.is_missing_value());
        assert!(!AxError::CannotComplete.is_missing_value());
    }

    #[test]
    fn display_mentions_code_for_unknown() {
        assert_eq!(AxError::Other(-7).to_string(), "accessibility error -7");
    }
}
