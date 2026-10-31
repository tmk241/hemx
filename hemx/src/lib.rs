//! Public authoring facade for hemx applications.
//!
//! Most application code should depend on this crate, use the proc-macros from
//! here, and import generated resources through `#[hemx::surface]`.

pub use hemx_core::{
    navigate, push, redirect, replace, CssClass, CssClasses, Effect, Form, FormContract,
    FormControlKind, FormError, FormField, FormModel, FormValue, FromForm, IntoEffect,
};

/// Generated metadata tokens used by macros, generated code, integrations, and tests.
///
/// These stay addressable for compatibility, but ordinary app code should use generated
/// helpers instead of naming raw handles, params, events, or component refs.
#[doc(hidden)]
pub use hemx_core::{ComponentRef, EventName, GeneratedTarget, Handle, ParamName};
pub use hemx_derive::{app, component, form, handler, surface};

/// Advanced/raw hemx primitives used by generated code, integrations, and tests.
///
/// Beginner-facing application code should prefer generated targets, generated
/// handles/forms/classes, `Html`, `IntoEffect`, and tuple composition.
pub mod advanced {
    pub use hemx_core::*;
}

/// Rendered HTML of one build, produced by the generated `render` helpers.
pub use hemx_core::Html;

/// A hemplate partial that carries the stable key for a generated keyed target.
///
/// Generated keyed target helpers use this to keep ordinary handler code at the
/// level of `ui::row.replace(row)` instead of `ui::row.replace(row.id, &row)`.
pub trait KeyedPartial {
    fn hemx_key(&self) -> String;
}

pub mod prelude {
    pub use crate::Html;
    pub use hemx_core::{
        navigate, push, redirect, replace, CssClass, CssClasses, Form, FormModel, FormValue,
        IntoEffect,
    };
    pub use hemx_derive::{app, component, form, handler, surface};
}

#[cfg(test)]
mod tests {
    #[test]
    fn rendered_html_keeps_its_markup_and_build() {
        use crate::prelude::*;

        let html = Html::__rendered("<p>hello</p>", crate::advanced::BuildFingerprint(7));
        assert_eq!(html.as_str(), "<p>hello</p>");
        assert_eq!(html.as_ref(), "<p>hello</p>");
        assert_eq!(html.to_string(), "<p>hello</p>");
        assert_eq!(html.build(), crate::advanced::BuildFingerprint(7));
        assert_eq!(html.into_string(), "<p>hello</p>");
    }
}
