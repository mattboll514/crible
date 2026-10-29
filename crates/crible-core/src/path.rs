// SPDX-License-Identifier: Apache-2.0

//! Path components taken from a configuration or from the API.
//!
//! A name that Crible joins to a root directory (`index`, `model`) names one
//! directory below it. Nothing forbids `..` or an absolute path in JSON, and a
//! joined path would then leave the root: the guard here is the one place that
//! decides what a name may be, for the binary and for the server.

use crate::error::{CribleError, ErrorCode};

/// Accepts `name` only as one path component.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] when `name` is empty, `.`, `..`, or holds a
/// separator (`/` or `\`).
pub fn path_component<'a>(name: &'a str, what: &str) -> Result<&'a str, CribleError> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err(CribleError::new(
            ErrorCode::InvalidRequest,
            format!("{what} '{name}' must be a single path component"),
        ));
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::path_component;
    use crate::error::ErrorCode;

    #[test]
    fn a_plain_name_is_accepted() {
        assert_eq!(
            path_component("legifacile", "index name").expect("a plain name is accepted"),
            "legifacile"
        );
        assert_eq!(
            path_component("a.b-c_d", "index name").expect("dots and dashes are component-safe"),
            "a.b-c_d"
        );
    }

    #[test]
    fn an_empty_name_and_a_dot_are_refused() {
        for name in ["", ".", ".."] {
            let error = path_component(name, "index name")
                .expect_err("an empty name, a dot or a dot-dot is refused");
            assert_eq!(error.code(), ErrorCode::InvalidRequest);
            assert!(error.message().contains(name), "got {}", error.message());
        }
    }

    #[test]
    fn a_separator_is_refused() {
        for name in ["a/b", "../model", "a\\b", "/etc/passwd"] {
            let error =
                path_component(name, "model name").expect_err("a name with a separator is refused");
            assert_eq!(error.code(), ErrorCode::InvalidRequest);
            assert!(error.message().contains(name), "got {}", error.message());
            assert!(
                error.message().contains("model name"),
                "got {}",
                error.message()
            );
        }
    }
}
