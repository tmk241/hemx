//! Public authoring facade for hemx applications.
//!
//! Most application code should depend on this crate, use the proc-macros from
//! here, and import generated resources through `#[hemx::surface]`.

use hemx_core::SafeHtml;
#[cfg(not(target_arch = "wasm32"))]
use hemx_core::{KeyedSlot, Slot};

pub use hemx_core::{
    navigate, push, redirect, replace, CssClass, CssClasses, Effect, Form, FormContract,
    FormControlKind, FormError, FormField, FormModel, FormValue, FromForm, IntoEffect,
};

/// Generated metadata tokens used by macros, generated code, integrations, and tests.
///
/// These stay addressable for compatibility, but ordinary app code should use generated
/// helpers instead of naming raw handles, params, events, atoms, or component refs.
#[doc(hidden)]
pub use hemx_core::{Atom, ComponentRef, EventName, GeneratedTarget, Handle, ParamName};
pub use hemx_derive::{app, component, form, handler, surface};

/// Browser/WASM integration used by `#[hemx::handler(client)]`.
///
/// The feature is opt-in so server-first applications do not compile or ship
/// WASM dependencies.
#[cfg(feature = "client")]
#[doc(hidden)]
pub use hemx_wasm as wasm;

/// Advanced/raw hemx primitives used by generated code, integrations, and tests.
///
/// Beginner-facing application code should prefer generated targets, generated
/// handles/forms/classes, `Html`, `IntoEffect`, and tuple composition.
pub mod advanced {
    pub use hemx_core::*;

    #[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
fn render_template(view: &impl hemplate::Hemplate) -> Html {
    let mut html = String::with_capacity(view.size_hint());
    view.render_into(&mut html).unwrap();
    __private::html_trusted(html)
}

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub fn render_html(view: &impl hemplate::Hemplate) -> Html {
    render_template(view)
}

/// Compatibility shim for raw slot rendering.
///
/// Prefer generated target objects such as `targets::list.put(&view)` so
/// generated resource lowering stays attached to the view boundary.
#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
impl<T> RenderSlotExt for Slot<T> {
    fn render(self, view: &impl hemplate::Hemplate) -> Effect {
        self.html(render_template(view))
    }
}

/// Compatibility shim for raw keyed slot rendering.
///
/// Prefer generated target objects such as `targets::row.append(&view)` so
/// generated resource lowering stays attached to the view boundary.
#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub trait RenderKeyedSlotExt<K> {
    fn append(self, key: K, view: &impl hemplate::Hemplate) -> Effect;
    fn prepend(self, key: K, view: &impl hemplate::Hemplate) -> Effect;
    fn replace(self, key: K, view: &impl hemplate::Hemplate) -> Effect;

    fn append_view(self, key: K, view: &impl hemplate::Hemplate) -> Effect
    where
        Self: Sized,
    {
        self.append(key, view)
    }

    fn prepend_view(self, key: K, view: &impl hemplate::Hemplate) -> Effect
    where
        Self: Sized,
    {
        self.prepend(key, view)
    }

    fn replace_view(self, key: K, view: &impl hemplate::Hemplate) -> Effect
    where
        Self: Sized,
    {
        self.replace(key, view)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<K, T> RenderKeyedSlotExt<K> for KeyedSlot<K, T>
where
    K: ToString,
{
    fn append(self, key: K, view: &impl hemplate::Hemplate) -> Effect {
        self.append_html(key, render_template(view))
    }

    fn prepend(self, key: K, view: &impl hemplate::Hemplate) -> Effect {
        self.prepend_html(key, render_template(view))
    }

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
        fn render_into(&self, out: &mut String) -> Result<(), hemplate::error::HemplateError> {
            out.push_str("<strong>ok</strong>");
            Ok(())
        }
    }

    struct HintTrackingView(Cell<bool>);

    impl hemplate::Hemplate for HintTrackingView {
        fn render_into(&self, out: &mut String) -> Result<(), hemplate::error::HemplateError> {
            out.push_str("<strong>hinted</strong>");
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
