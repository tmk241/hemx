#![doc = include_str!("../README.md")]

#[cfg(feature = "axum")]
pub mod axum;
mod html;
mod process;

pub use html::{HtmlElement, HtmlInspectionError, HtmlInspector, HtmlSelection};
pub use process::{ProcessError, ProcessFailure, TestProcess, TestProcessBuilder};

use hemx_core::{
    BuildFingerprint, Effect, EffectBatch, Form, GeneratedTarget, IntoEffect, KeyedSlot,
    ResourceId, ResourceRef, ScopeKey, Slot,
};
use std::future::Future;

/// Run a synchronous handler with typed input and inspect its effects.
pub fn run<I, F, R>(handler: F, input: I) -> EffectInspector
where
    F: FnOnce(I) -> R,
    R: IntoEffect,
{
    inspect(handler(input))
}

/// Run an asynchronous handler with typed input and inspect its effects.
pub async fn run_async<I, F, HandlerFuture, R>(handler: F, input: I) -> EffectInspector
where
    F: FnOnce(I) -> HandlerFuture,
    HandlerFuture: Future<Output = R>,
    R: IntoEffect,
{
    inspect(handler(input).await)
}

/// Run a fallible synchronous handler without hiding its concrete error.
///
/// Only the successful value is converted into effects. `IntoEffect`
/// conversion is infallible in the current public contract, so an error from
/// the handler is returned unchanged rather than becoming an empty or
/// success-looking batch.
pub fn run_result<I, F, R, E>(handler: F, input: I) -> Result<EffectInspector, E>
where
    F: FnOnce(I) -> Result<R, E>,
    R: IntoEffect,
{
    handler(input).map(inspect)
}

/// Run a fallible asynchronous handler without hiding its concrete error.
///
/// Only the successful value is converted into effects; the handler's original
/// error type and value are preserved.
pub async fn run_async_result<I, F, HandlerFuture, R, E>(
    handler: F,
    input: I,
) -> Result<EffectInspector, E>
where
    F: FnOnce(I) -> HandlerFuture,
    HandlerFuture: Future<Output = Result<R, E>>,
    R: IntoEffect,
{
    handler(input).await.map(inspect)
}

fn inspection_fingerprint() -> BuildFingerprint {
    BuildFingerprint(0)
}

pub fn inspect(effect: impl IntoEffect) -> EffectInspector {
    inspect_batch(effect.into_batch(inspection_fingerprint()))
}

/// Parse and structurally inspect a complete server-rendered HTML document.
pub fn inspect_html_document(html: impl Into<String>) -> HtmlInspector {
    HtmlInspector::document(html.into(), String::from("HTML document"))
}

/// Parse and structurally inspect a server-rendered HTML fragment.
pub fn inspect_html_fragment(html: impl Into<String>) -> HtmlInspector {
    HtmlInspector::fragment(html.into(), String::from("HTML fragment"))
}

/// Inspect an already-dispatched batch without matching raw effect variants in tests.
pub fn inspect_batch(batch: EffectBatch) -> EffectInspector {
    EffectInspector { batch }
}

/// Decode and inspect an effect wire response without exposing `EffectBatch` in tests.
pub fn inspect_wire(bytes: &[u8]) -> EffectInspector {
    try_inspect_wire(bytes)
        .unwrap_or_else(|error| panic!("invalid hemx effect wire response: {error:?}"))
}

/// Try to decode and inspect an effect wire response.
pub fn try_inspect_wire(bytes: &[u8]) -> Result<EffectInspector, hemx_core::WireError> {
    EffectBatch::from_wire(bytes).map(inspect_batch)
}

/// Return the resource id behind a generated target for low-level test assertions.
pub fn target_resource(target: impl GeneratedTarget) -> ResourceId {
    target.__hemx_resource_id()
}

/// Return the unscoped resource reference behind a generated target for low-level test assertions.
pub fn target_ref(target: impl GeneratedTarget) -> ResourceRef {
    ResourceRef::unscoped(target_resource(target))
}

/// Build an interaction request body from a generated handle and form fields.
pub fn handle_form_body<I>(handle: hemx_core::Handle<I>, fields: &[(&str, &str)]) -> String {
    let mut body = form_pair("__h", &handle.to_string());
    for (name, value) in fields {
        body.push('&');
        body.push_str(&form_pair(name, value));
    }
    body
}

/// Build a request body for invalid-handle tests without exposing the wire field name.
pub fn unknown_handle_form_body(id: u32) -> String {
    form_pair("__h", &id.to_string())
}

fn form_pair(name: &str, value: &str) -> String {
    format!("{}={}", form_encode(name), form_encode(value))
}

fn form_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[derive(Clone, Debug)]
pub struct EffectInspector {
    batch: EffectBatch,
}

impl EffectInspector {
    pub fn batch(&self) -> &EffectBatch {
        &self.batch
    }

    pub fn ops(&self) -> &[Effect] {
        &self.batch.ops
    }

    pub fn contains(&self, op: &Effect) -> bool {
        self.batch.ops.contains(op)
    }

    pub fn op_count(&self) -> usize {
        self.batch.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.batch.ops.is_empty()
    }

    pub fn has_resource(&self, resource: ResourceId) -> bool {
        self.batch
            .ops
            .iter()
            .any(|op| op_targets_resource(op, resource))
    }

    /// Assert against the same generated target object application handlers use.
    pub fn has_target(&self, target: impl GeneratedTarget) -> bool {
        self.has_resource(target.__hemx_resource_id())
    }

    /// Check that a generated target receives a text update, without matching raw effects.
    pub fn updates_text(&self, target: impl GeneratedTarget) -> bool {
        self.has_text_update_containing(target.__hemx_resource_id(), "")
    }

    /// Check that a generated target receives a text update containing a fragment.
    ///
    /// This associates the payload condition with the intended target, unlike a separate global
    /// [`Self::payload_contains`] check that can accidentally match another operation.
    pub fn updates_text_containing(&self, target: impl GeneratedTarget, needle: &str) -> bool {
        self.has_text_update_containing(target.__hemx_resource_id(), needle)
    }

    /// Assert that a generated target receives a text update containing a fragment.
    #[track_caller]
    pub fn assert_updates_text_containing(&self, target: impl GeneratedTarget, needle: &str) {
        let resource = target.__hemx_resource_id();
        assert!(
            self.has_text_update_containing(resource, needle),
            "expected a text update for {resource:?} containing {needle:?}; actual effects: {:#?}",
            self.batch.ops
        );
    }

    fn has_text_update_containing(&self, resource: ResourceId, needle: &str) -> bool {
        self.batch.ops.iter().any(|op| {
            matches!(
                op,
                Effect::Patch { target, html, .. }
                    if target.resource == resource && html.as_str().contains(needle)
            )
        })
    }

    /// Assert that a generated target receives an HTML update, without matching raw effects.
    pub fn updates_html(&self, target: impl GeneratedTarget) -> bool {
        let resource = target.__hemx_resource_id();
        self.batch.ops.iter().any(|op| {
            matches!(
                op,
                Effect::Patch { target, .. } | Effect::Insert { target, .. }
                    if target.resource == resource
            )
        })
    }

    /// Check that a generated target receives an HTML update containing text.
    pub fn updates_html_containing(&self, target: impl GeneratedTarget, needle: &str) -> bool {
        self.has_html_update_containing(target.__hemx_resource_id(), needle)
    }

    /// Assert that a generated target receives an HTML update containing text.
    ///
    /// Unlike wrapping [`Self::updates_html_containing`] in `assert!`, failures include the
    /// expected resource and payload fragment together with every actual effect operation.
    #[track_caller]
    pub fn assert_updates_html_containing(&self, target: impl GeneratedTarget, needle: &str) {
        let resource = target.__hemx_resource_id();
        assert!(
            self.has_html_update_containing(resource, needle),
            "expected an HTML update for {resource:?} containing {needle:?}; actual effects: {:#?}",
            self.batch.ops
        );
    }

    fn has_html_update_containing(&self, resource: ResourceId, needle: &str) -> bool {
        self.batch.ops.iter().any(|op| {
            matches!(
                op,
                Effect::Patch { target, html, .. } | Effect::Insert { target, html, .. }
                    if target.resource == resource && html.as_str().contains(needle)
            )
        })
    }

    /// Parse the single HTML effect for a generated target as a complete document.
    ///
    /// Returns an error when no HTML operation targets the resource or when more
    /// than one operation would make the selected payload ambiguous.
    pub fn target_html_document(
        &self,
        target: impl GeneratedTarget,
    ) -> Result<HtmlInspector, HtmlInspectionError> {
        let resource = target.__hemx_resource_id();
        let (html, operation) = self.single_target_html(resource)?;
        Ok(HtmlInspector::document(
            html.to_owned(),
            format!("{operation} HTML effect for generated target {resource:?}"),
        ))
    }

    /// Parse the single HTML effect for a generated target as a fragment.
    ///
    /// `Put`, `Insert`, and `Prepend` HTML payloads are supported. The parser is
    /// kept internal; the returned inspector and all selected elements own their
    /// observable data.
    pub fn target_html_fragment(
        &self,
        target: impl GeneratedTarget,
    ) -> Result<HtmlInspector, HtmlInspectionError> {
        let resource = target.__hemx_resource_id();
        let (html, operation) = self.single_target_html(resource)?;
        Ok(HtmlInspector::fragment(
            html.to_owned(),
            format!("{operation} HTML effect for generated target {resource:?}"),
        ))
    }

    fn single_target_html(
        &self,
        resource: ResourceId,
    ) -> Result<(&str, &'static str), HtmlInspectionError> {
        let target_effects = self
            .batch
            .ops
            .iter()
            .filter(|effect| op_targets_resource(effect, resource))
            .collect::<Vec<_>>();
        let html_effects = target_effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Patch { html, .. } => Some((html.as_str(), "Patch")),
                Effect::Insert { html, .. } => Some((html.as_str(), "Insert")),
                _ => None,
            })
            .collect::<Vec<_>>();

        match html_effects.as_slice() {
            [only] => Ok(*only),
            [] => {
                let actual = if target_effects.is_empty() {
                    format!("all effects: {:#?}", self.batch.ops)
                } else {
                    format!("effects for target: {target_effects:#?}")
                };
                Err(HtmlInspectionError::new(format!(
                    "expected one HTML effect for generated target {resource:?}, but found none; {actual}"
                )))
            }
            many => Err(HtmlInspectionError::new(format!(
                "expected one HTML effect for generated target {resource:?}, but found {} and cannot choose a document or fragment payload; effects for target: {target_effects:#?}",
                many.len()
            ))),
        }
    }

    /// Assert that a keyed generated target is replaced with HTML containing text.
    pub fn replaces_keyed_html_containing(
        &self,
        target: impl GeneratedTarget,
        key: impl ToString,
        needle: &str,
    ) -> bool {
        let resource = target.__hemx_resource_id();
        let scope = Some(ScopeKey::KeyValue(key.to_string()));
        self.batch.ops.iter().any(|op| {
            matches!(
                op,
                Effect::Patch { target, html, .. }
                    if target.resource == resource && target.scope == scope && html.as_str().contains(needle)
            )
        })
    }

    /// Assert that a keyed generated target appends HTML containing text.
    pub fn inserts_html_containing(
        &self,
        target: impl GeneratedTarget,
        key: impl ToString,
        needle: &str,
    ) -> bool {
        let resource = target.__hemx_resource_id();
        let key_marker = format!("data-hemx-key=\"{}\"", key.to_string());
        self.batch.ops.iter().any(|op| {
            matches!(
                op,
                Effect::Insert { target, html, .. }
                    if target.resource == resource
                        && html.as_str().contains(&key_marker)
                        && html.as_str().contains(needle)
            )
        })
    }

    /// Assert that a keyed generated target removes a key.
    pub fn removes_key(&self, target: impl GeneratedTarget, key: impl ToString) -> bool {
        let resource = target.__hemx_resource_id();
        let key = key.to_string();
        self.batch.ops.iter().any(|op| {
            matches!(
                op,
                Effect::Remove { target }
                    if target.resource == resource && target.scope == Some(ScopeKey::KeyValue(key.clone()))
            )
        })
    }

    /// Assert that the batch requests a push navigation to a URL.
    pub fn pushes_to(&self, url: &str) -> bool {
        self.batch.ops.iter().any(|op| {
            matches!(
                op,
                Effect::Visit { url: actual_url, .. } if actual_url == url
            )
        })
    }

    /// Assert that any payload or URL contains text, without matching raw effects.
    pub fn payload_contains(&self, needle: &str) -> bool {
        self.batch
            .ops
            .iter()
            .any(|op| effect_payload_contains(op, needle))
    }

    /// Assert that no payload or URL contains text, without matching raw effects.
    pub fn payload_excludes(&self, needle: &str) -> bool {
        self.batch
            .ops
            .iter()
            .all(|op| !effect_payload_contains(op, needle))
    }

    /// Assert that generated keyed-row metadata for a key is absent from payloads.
    pub fn payload_excludes_key(&self, key: impl ToString) -> bool {
        self.payload_excludes(&format!("data-hemx-key=\"{}\"", key.to_string()))
    }

    /// Return HTML for a generated target containing text, without exposing raw payloads.
    pub fn target_html_containing(
        &self,
        target: impl GeneratedTarget,
        needle: &str,
    ) -> Option<&str> {
        let resource = target.__hemx_resource_id();
        self.batch.ops.iter().find_map(|op| match op {
            Effect::Patch { target, html, .. }
                if target.resource == resource && html.as_str().contains(needle) =>
            {
                Some(html.as_str())
            }
            Effect::Insert { target, html, .. }
                if target.resource == resource && html.as_str().contains(needle) =>
            {
                Some(html.as_str())
            }
            _ => None,
        })
    }

    /// Assert that a named generated event is emitted with the exact payload.
    pub fn emits(&self, name: &str, payload: &str) -> bool {
        self.batch.ops.iter().any(|op| {
            matches!(
                op,
                Effect::Dispatch { event: actual_name, payload: actual_payload }
                    if actual_name == name && actual_payload.as_slice() == payload.as_bytes()
            )
        })
    }

    /// Assert that a named generated event payload contains text.
    pub fn emits_containing(&self, name: &str, needle: &str) -> bool {
        self.batch.ops.iter().any(|op| {
            matches!(
                op,
                Effect::Dispatch { event: actual_name, payload }
                    if actual_name == name && String::from_utf8_lossy(payload).contains(needle)
            )
        })
    }

    pub fn has_ref(&self, target: &ResourceRef) -> bool {
        self.batch.ops.iter().any(|op| op_targets_ref(op, target))
    }

    pub fn has_slot<T>(&self, slot: Slot<T>) -> bool {
        self.has_resource(slot.id())
    }

    pub fn has_keyed_slot<K, T>(&self, slot: KeyedSlot<K, T>) -> bool
    where
        K: ToString,
    {
        self.has_resource(slot.id())
    }

    pub fn has_form<T>(&self, form: Form<T>) -> bool {
        self.has_resource(form.id())
    }
}

fn effect_payload_contains(op: &Effect, needle: &str) -> bool {
    match op {
        Effect::Patch { html, .. } | Effect::Insert { html, .. } => html.as_str().contains(needle),
        Effect::Dispatch { payload, .. } => String::from_utf8_lossy(payload).contains(needle),
        Effect::Visit { url, .. } => url.contains(needle),
        Effect::Remove { .. }
        | Effect::Move { .. }
        | Effect::Focus { .. }
        | Effect::Scroll { .. } => false,
    }
}

fn op_targets_resource(op: &Effect, resource: ResourceId) -> bool {
    match op {
        Effect::Patch { target, .. }
        | Effect::Insert { target, .. }
        | Effect::Remove { target }
        | Effect::Move { target, .. }
        | Effect::Focus { target, .. }
        | Effect::Scroll { target, .. } => target.resource == resource,
        Effect::Visit { .. } | Effect::Dispatch { .. } => false,
    }
}

fn op_targets_ref(op: &Effect, wanted: &ResourceRef) -> bool {
    match op {
        Effect::Patch { target, .. }
        | Effect::Insert { target, .. }
        | Effect::Remove { target }
        | Effect::Move { target, .. }
        | Effect::Focus { target, .. }
        | Effect::Scroll { target, .. } => target == wanted,
        Effect::Visit { .. } | Effect::Dispatch { .. } => false,
    }
}
