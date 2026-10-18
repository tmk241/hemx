//! Public authoring facade for hemx applications.
//!
//! Most application code should depend on this crate, use the proc-macros from
//! here, and import generated resources through `#[hemx::surface]`.

use hemx_core::{KeyedSlot, SafeHtml, Slot};

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

    pub fn render(view: &impl hemplate::Hemplate) -> crate::Html {
        crate::render_template(view)
    }
}

/// Rendered, checked HTML produced by hemplate/hemx rendering helpers.
///
/// Raw trusted HTML construction remains an advanced boundary; beginner-facing
/// code should receive `Html` values from generated render helpers.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Html(SafeHtml);

impl Html {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn into_string(self) -> String {
        self.0.into_string()
    }

    pub fn join(fragments: impl IntoIterator<Item = Html>) -> Self {
        Self(SafeHtml::join(fragments.into_iter().map(Into::into)))
    }
}

impl From<Html> for SafeHtml {
    fn from(value: Html) -> Self {
        value.0
    }
}

impl AsRef<str> for Html {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl core::fmt::Display for Html {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[doc(hidden)]
pub mod __private {
    use super::{Html, SafeHtml};

    pub fn html_trusted(value: impl Into<String>) -> Html {
        Html(SafeHtml::trusted(value))
    }
}

fn render_template(view: &impl hemplate::Hemplate) -> Html {
    let mut html = String::with_capacity(view.size_hint());
    view.render_into(&mut html).unwrap();
    __private::html_trusted(html)
}

pub fn page(view: &impl hemplate::Hemplate) -> Html {
    render_template(view)
}

/// A hemplate partial that carries the stable key for a generated keyed target.
///
/// Generated keyed target helpers use this to keep ordinary handler code at the
/// level of `ui::row.replace(row)` instead of `ui::row.replace(row.id, &row)`.
pub trait KeyedPartial {
    fn hemx_key(&self) -> String;
}

#[doc(hidden)]
pub fn render_html(view: &impl hemplate::Hemplate) -> Html {
    render_template(view)
}

/// Compatibility shim for raw slot rendering.
///
/// Prefer generated target objects such as `targets::list.put(&view)` so
/// generated resource lowering stays attached to the view boundary.
#[doc(hidden)]
pub trait RenderSlotExt {
    fn render(self, view: &impl hemplate::Hemplate) -> Effect;

    fn render_view(self, view: &impl hemplate::Hemplate) -> Effect
    where
        Self: Sized,
    {
        self.render(view)
    }
}

impl<T> RenderSlotExt for Slot<T> {
    fn render(self, view: &impl hemplate::Hemplate) -> Effect {
        self.html(render_template(view))
    }
}

/// Render into an existing keyed row. Collection membership and order belong to a Slot's Rows.
///
/// Prefer generated target objects such as `targets::row.replace(&view)` so
/// generated resource lowering stays attached to the view boundary.
#[doc(hidden)]
pub trait RenderKeyedSlotExt<K> {
    fn replace(self, key: K, view: &impl hemplate::Hemplate) -> Effect;

    fn replace_view(self, key: K, view: &impl hemplate::Hemplate) -> Effect
    where
        Self: Sized,
    {
        self.replace(key, view)
    }
}

impl<K, T> RenderKeyedSlotExt<K> for KeyedSlot<K, T>
where
    K: ToString,
{
    fn replace(self, key: K, view: &impl hemplate::Hemplate) -> Effect {
        self.replace_html(key, render_template(view))
    }
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
    use std::cell::Cell;

    struct InlineView;

    impl hemplate::Hemplate for InlineView {
        fn render_into(&self, out: &mut dyn core::fmt::Write) -> Result<(), hemplate::RenderError> {
            out.write_str("<strong>ok</strong>")?;
            Ok(())
        }
    }

    struct HintTrackingView(Cell<bool>);

    impl hemplate::Hemplate for HintTrackingView {
        fn render_into(&self, out: &mut dyn core::fmt::Write) -> Result<(), hemplate::RenderError> {
            out.write_str("<strong>hinted</strong>")?;
            Ok(())
        }

        fn size_hint(&self) -> usize {
            self.0.set(true);
            23
        }
    }

    #[test]
    fn trusted_render_path_uses_the_view_size_hint() {
        let view = HintTrackingView(Cell::new(false));

        assert_eq!(crate::page(&view).as_str(), "<strong>hinted</strong>");
        assert!(view.0.get(), "trusted render path must consult size_hint");
    }

    #[test]
    fn page_is_the_short_safe_html_helper() {
        assert_eq!(crate::page(&InlineView).as_str(), "<strong>ok</strong>");
        assert_eq!(
            crate::render_html(&InlineView).as_str(),
            "<strong>ok</strong>"
        );
        assert_eq!(
            crate::advanced::render(&InlineView).as_str(),
            "<strong>ok</strong>"
        );
    }

    #[test]
    fn prelude_exports_html_for_page_composition() {
        use crate::prelude::*;

        let html = Html::join([crate::page(&InlineView)]);

        assert_eq!(html.as_str(), "<strong>ok</strong>");
    }

    #[test]
    fn html_string_views_preserve_the_rendered_fragment() {
        let html = crate::__private::html_trusted("<p>hello</p>");
        assert_eq!(html.as_str(), "<p>hello</p>");
        assert_eq!(html.as_ref(), "<p>hello</p>");
        assert_eq!(html.to_string(), "<p>hello</p>");
        assert_eq!(html.into_string(), "<p>hello</p>");
    }
}
