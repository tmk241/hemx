#![doc = include_str!("../README.md")]

#[cfg(feature = "axum")]
pub mod axum;
mod html;

pub use html::{HtmlElement, HtmlInspectionError, HtmlInspector, HtmlSelection};

use hemx_core::{
    Atom, BuildFingerprint, Effect, EffectBatch, Form, GeneratedTarget, IntoEffect, KeyedSlot,
    NavigateMode, Payload, ResourceId, ResourceKind, ResourceRef, ScopeKey, Slot,
};
use std::future::Future;
use std::io;
use std::net::TcpStream;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// A child process owned by an integration test and proven ready over TCP.
///
/// The process is killed and reaped on every return path, including panics. Startup failures name
/// the process and address and distinguish early exit from a readiness timeout.
pub struct TestProcess {
    child: Child,
}

fn test_process_try_wait(child: &mut Child) -> io::Result<Option<ExitStatus>> {
    child.try_wait()
}

fn test_process_poll_delay() {
    std::thread::sleep(Duration::from_millis(25));
}

impl TestProcess {
    pub fn start(
        mut command: Command,
        label: impl Into<String>,
        addr: &str,
        timeout: Duration,
    ) -> io::Result<Self> {
        let label = label.into();
        let child = command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                io::Error::new(error.kind(), format!("failed to spawn {label}: {error}"))
            })?;
        let mut process = Self { child };
        let deadline = Instant::now() + timeout;
        loop {
            if TcpStream::connect(addr).is_ok() {
                return Ok(process);
            }
            if let Some(status) = test_process_try_wait(&mut process.child)? {
                return Err(io::Error::other(format!(
                    "{label} exited with {status} before listening on {addr}"
                )));
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("timed out after {timeout:?} waiting for {label} to listen on {addr}"),
                ));
            }
            test_process_poll_delay();
        }
    }
}

impl Drop for TestProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

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

/// Build a browser-driver selector from a generated handle without exposing runtime ids in tests.
pub fn handle_selector<I>(handle: hemx_core::Handle<I>) -> String {
    attr_selector("data-hid", &handle.to_string())
}

/// Build a selector for a clickable button with a generated handle.
pub fn handle_button_selector<I>(handle: hemx_core::Handle<I>) -> String {
    format!("button{}", handle_selector(handle))
}

/// Build a selector for a heading in a semantic container without spelling document structure in examples.
pub fn heading_selector(scope_selector: &str, level: u8) -> String {
    assert!((1..=6).contains(&level), "heading level must be 1..=6");
    if scope_selector.is_empty() {
        format!("h{level}")
    } else {
        format!("{scope_selector} h{level}")
    }
}

/// Build a selector for article content without spelling document structure in examples.
pub fn article_selector() -> &'static str {
    "article"
}

/// Build a selector for emphasized/card-title text without spelling document structure.
pub fn strong_text_selector() -> &'static str {
    "strong"
}

/// Build a selector for secondary/help text without spelling document structure.
pub fn small_text_selector() -> &'static str {
    "small"
}

/// Build a selector for an HTML tag that must be absent when user text is escaped.
pub fn escaped_markup_selector(tag: &str) -> String {
    assert!(
        tag.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-'),
        "tag selector must be a simple tag name"
    );
    tag.to_owned()
}

/// Build a selector for list items without spelling document structure in examples.
pub fn list_item_selector(scope_selector: &str) -> String {
    if scope_selector.is_empty() {
        "li".to_owned()
    } else {
        format!("{scope_selector} li")
    }
}

/// Build a selector for prose text in a semantic container without spelling document structure.
pub fn prose_selector(scope_selector: &str) -> String {
    if scope_selector.is_empty() {
        "p".to_owned()
    } else {
        format!("{scope_selector} p")
    }
}

/// Build a selector for a form in a semantic container without spelling form structure in examples.
pub fn form_selector(scope_selector: &str) -> String {
    format!("{scope_selector} form")
}

/// Build a selector for a form select's options from the authoring field name.
pub fn select_options_selector(field: &str) -> String {
    format!("select{} > option", attr_selector("name", field))
}

/// Build a selector for an app-owned semantic class.
pub fn class_selector(class: &str) -> String {
    assert_simple_selector_part(class, "class");
    format!(".{class}")
}

/// Build a selector for an element carrying an app-owned semantic class.
pub fn element_class_selector(element: &str, class: &str) -> String {
    assert_simple_selector_part(element, "element");
    assert_simple_selector_part(class, "class");
    format!("{element}.{class}")
}

/// Build a selector for classed children inside an app-owned semantic container.
pub fn class_child_selector(parent_class: &str, element: &str, class: &str) -> String {
    assert_simple_selector_part(parent_class, "parent class");
    assert_simple_selector_part(element, "element");
    assert_simple_selector_part(class, "class");
    format!(".{parent_class} > {element}.{class}")
}

/// Build a selector for an element inside an app-owned semantic class.
pub fn class_descendant_selector(parent_class: &str, element: &str) -> String {
    assert_simple_selector_part(parent_class, "parent class");
    assert_simple_selector_part(element, "element");
    format!(".{parent_class} {element}")
}

/// Build a selector for disabled action buttons without spelling CSS selector state in examples.
pub fn disabled_button_selector() -> &'static str {
    "button[disabled]"
}

/// Build a selector for progressive-enhancement navigation links.
pub fn nav_link_selector(href: &str) -> String {
    format!("a{}", attr_selector("href", href))
}

/// Build a selector for page-enhanced navigation links that do not use handler dispatch.
pub fn page_nav_link_selector(href: &str) -> String {
    format!(
        "{}[data-hemx-nav]:not([data-hemx-handle])",
        nav_link_selector(href)
    )
}

/// Build a browser-driver selector from a generated target without exposing runtime ids in tests.
pub fn target_selector(target: impl GeneratedTarget) -> String {
    let resource = target.__hemx_resource_id();
    let attr = runtime_resource_attr(resource.kind);
    attr_selector(attr, &resource.id.to_string())
}

/// Assert rendered HTML contains a generated target and report the generated name on failure.
pub fn assert_rendered_target(target: impl GeneratedTarget, generated_name: &str, html: &str) {
    let resource = target.__hemx_resource_id();
    let marker = runtime_attr_marker(
        runtime_resource_attr(resource.kind),
        &resource.id.to_string(),
    );
    assert!(
        html.contains(&marker),
        "rendered hemx target `{generated_name}` missing runtime marker {marker}"
    );
}

/// Assert rendered HTML contains a generated handle and report the generated name on failure.
pub fn assert_rendered_handle<I>(handle: hemx_core::Handle<I>, generated_name: &str, html: &str) {
    let marker = runtime_attr_marker("data-hid", &handle.to_string());
    assert!(
        html.contains(&marker),
        "rendered hemx handle `{generated_name}` missing runtime marker {marker}"
    );
}

/// Build a selector for an hemx root from its authoring name.
pub fn root_selector(name: &str) -> String {
    attr_selector("data-hemx-root", name)
}

/// Build a selector for a specific root element from its authoring name.
pub fn root_element_selector(element: &str, name: &str) -> String {
    format!("{}{}", element, root_selector(name))
}

/// Build a selector for the document body without spelling raw document structure in examples.
pub fn document_body_selector() -> &'static str {
    "body"
}

/// Build a selector for the document title without spelling raw document structure in examples.
pub fn document_title_selector() -> &'static str {
    "title"
}

/// Build a selector for the hemx runtime script without exposing its asset path in tests.
pub fn runtime_script_selector() -> &'static str {
    "script[src^=\"/hemx.\"][src$=\".js\"]"
}

/// Build a selector for any hemx root without spelling the attribute in tests.
pub fn any_root_selector() -> &'static str {
    "[data-hemx-root]"
}

/// Build a selector for a keyed generated row without spelling runtime key metadata.
pub fn keyed_selector(base_selector: &str, key: impl ToString) -> String {
    format!(
        "{}{}",
        base_selector,
        attr_selector("data-key", &key.to_string())
    )
}

/// Build a selector for all generated keyed rows under a semantic base selector.
pub fn keyed_items_selector(base_selector: &str) -> String {
    format!("{base_selector}[data-key]")
}

/// Build a selector for an island from its authoring name.
pub fn island_selector(name: &str) -> String {
    attr_selector("data-hemx-island", name)
}

/// Return the island metadata attribute name without spelling it in product tests.
pub fn island_attribute_name() -> &'static str {
    "data-hemx-island"
}

/// Return the runtime island event name for an authoring island name.
pub fn island_event_name(name: &str) -> String {
    format!("hemx:island-{name}")
}

/// Return an SSE enhancement marker without spelling framework metadata in tests.
pub fn sse_endpoint_marker(path: &str) -> String {
    format!("data-hemx-sse=\"{path}\"")
}

/// Return the island snapshot marker without spelling island metadata in tests.
pub fn island_snapshot_marker() -> &'static str {
    "data-island-snapshot="
}

/// Build a selector for island readouts without spelling island metadata in tests.
pub fn island_readout_selector() -> &'static str {
    "[data-island-readout]"
}

/// Build browser-driver JavaScript for injecting a synthetic island probe.
///
/// This lets product tests exercise the island bridge without spelling hemx island
/// metadata attributes in the test body.
pub fn island_probe_script(
    element_id: &str,
    island_name: &str,
    snapshot: &str,
    event_detail: &str,
) -> String {
    format!(
        r#"
const root = arguments[0];
const islandName = {island_name};
const island = document.createElement('article');
island.id = {element_id};
island.setAttribute('data-hemx-island', islandName);
island.setAttribute('data-island-snapshot', {snapshot});

const canvas = document.createElement('canvas');
canvas.width = 32;
canvas.height = 16;
island.appendChild(canvas);

const readout = document.createElement('p');
readout.setAttribute('data-island-readout', '');
readout.textContent = 'waiting';
island.appendChild(readout);

root.appendChild(island);
setTimeout(() => {{
  root.dispatchEvent(new CustomEvent('hemx:island-' + islandName, {{ bubbles: true, detail: {event_detail} }}));
}}, 25);
return true;
"#,
        element_id = js_string(element_id),
        island_name = js_string(island_name),
        snapshot = js_string(snapshot),
        event_detail = js_string(event_detail),
    )
}

/// Build a scoped selector for island readouts without spelling island metadata in tests.
pub fn scoped_island_readout_selector(scope_selector: &str) -> String {
    format!("{scope_selector} {}", island_readout_selector())
}

fn assert_simple_selector_part(value: &str, label: &str) {
    assert!(
        !value.is_empty()
            && value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-'),
        "{label} selector part must contain one or more ascii alphanumerics or '-'"
    );
}

fn runtime_resource_attr(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Slot => "data-sid",
        ResourceKind::Atom => "data-aid",
        ResourceKind::Handle => "data-hid",
        ResourceKind::Form => "data-fid",
    }
}

fn runtime_attr_marker(name: &str, value: &str) -> String {
    format!(r#"{name}="{value}""#)
}

fn attr_selector(name: &str, value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!(r#"[{name}="{escaped}"]"#)
}

fn js_string(value: &str) -> String {
    let mut escaped = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
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
                Effect::Put {
                    target,
                    payload: Payload::Text(text),
                } if target.resource == resource && text.contains(needle)
            )
        })
    }

    /// Assert that a generated target receives an HTML update, without matching raw effects.
    pub fn updates_html(&self, target: impl GeneratedTarget) -> bool {
        let resource = target.__hemx_resource_id();
        self.batch.ops.iter().any(|op| {
            matches!(
                op,
                Effect::Put {
                    target,
                    payload: Payload::Html(_),
                } if target.resource == resource
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
                Effect::Put {
                    target,
                    payload: Payload::Html(html),
                } if target.resource == resource && html.contains(needle)
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
                Effect::Put {
                    payload: Payload::Html(html),
                    ..
                } => Some((html.as_str(), "Put")),
                Effect::Insert {
                    payload: Payload::Html(html),
                    ..
                } => Some((html.as_str(), "Insert")),
                Effect::Prepend {
                    payload: Payload::Html(html),
                    ..
                } => Some((html.as_str(), "Prepend")),
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
                Effect::Put {
                    target,
                    payload: Payload::Html(html),
                } if target.resource == resource && target.scope == scope && html.contains(needle)
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
        let key = key.to_string();
        self.batch.ops.iter().any(|op| {
            matches!(
                op,
                Effect::Insert {
                    target,
                    key: actual_key,
                    payload: Payload::Html(html),
                } if target.resource == resource && actual_key == &key && html.contains(needle)
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
                Effect::Remove {
                    target,
                    key: Some(actual_key),
                } if target.resource == resource && actual_key == &key
            )
        })
    }

    /// Assert that the batch requests a push navigation to a URL.
    pub fn pushes_to(&self, url: &str) -> bool {
        self.batch.ops.iter().any(|op| {
            matches!(
                op,
                Effect::Navigate {
                    url: actual_url,
                    mode: NavigateMode::Push,
                    ..
                } if actual_url == url
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
        self.payload_excludes(&format!("data-key=\"{}\"", key.to_string()))
    }

    /// Return HTML for a generated target containing text, without exposing raw payloads.
    pub fn target_html_containing(
        &self,
        target: impl GeneratedTarget,
        needle: &str,
    ) -> Option<&str> {
        let resource = target.__hemx_resource_id();
        self.batch.ops.iter().find_map(|op| match op {
            Effect::Put {
                target,
                payload: Payload::Html(html),
            } if target.resource == resource && html.contains(needle) => Some(html.as_str()),
            Effect::Insert {
                target,
                payload: Payload::Html(html),
                ..
            } if target.resource == resource && html.contains(needle) => Some(html.as_str()),
            Effect::Prepend {
                target,
                payload: Payload::Html(html),
                ..
            } if target.resource == resource && html.contains(needle) => Some(html.as_str()),
            _ => None,
        })
    }

    /// Assert that a named generated event is emitted with the exact payload.
    pub fn emits(&self, name: &str, payload: &str) -> bool {
        self.batch.ops.iter().any(|op| {
            matches!(
                op,
                Effect::Emit {
                    name: actual_name,
                    payload: actual_payload,
                } if actual_name == name && actual_payload == payload
            )
        })
    }

    /// Assert that a named generated event payload contains text.
    pub fn emits_containing(&self, name: &str, needle: &str) -> bool {
        self.batch.ops.iter().any(|op| {
            matches!(
                op,
                Effect::Emit {
                    name: actual_name,
                    payload,
                } if actual_name == name && payload.contains(needle)
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

    pub fn has_atom<T>(&self, atom: Atom<T>) -> bool {
        self.has_resource(atom.id())
    }

    pub fn has_form<T>(&self, form: Form<T>) -> bool {
        self.has_resource(form.id())
    }

    /// Assert that a generated form is reset/cleared without matching raw events in tests.
    pub fn resets_form<T>(&self, form: Form<T>) -> bool {
        let form_id = form.id().id.to_string();
        self.batch.ops.iter().any(|op| {
            matches!(
                op,
                Effect::Emit { name, payload }
                    if name == "hemx:form-reset" && payload == &form_id
            )
        })
    }
}

fn effect_payload_contains(op: &Effect, needle: &str) -> bool {
    match op {
        Effect::Put { payload, .. }
        | Effect::Insert { payload, .. }
        | Effect::Prepend { payload, .. } => payload_value(payload).contains(needle),
        Effect::Emit { payload, .. } => payload.contains(needle),
        Effect::Navigate { url, .. } => url.contains(needle),
        Effect::Remove { .. } | Effect::Move { .. } | Effect::Focus { .. } => false,
    }
}

fn payload_value(payload: &Payload) -> &str {
    match payload {
        Payload::Text(value) | Payload::Html(value) => value,
    }
}

fn op_targets_resource(op: &Effect, resource: ResourceId) -> bool {
    match op {
        Effect::Put { target, .. }
        | Effect::Insert { target, .. }
        | Effect::Prepend { target, .. }
        | Effect::Remove { target, .. }
        | Effect::Move { target, .. }
        | Effect::Focus { target } => target.resource == resource,
        Effect::Navigate { .. } | Effect::Emit { .. } => false,
    }
}

fn op_targets_ref(op: &Effect, wanted: &ResourceRef) -> bool {
    match op {
        Effect::Put { target, .. }
        | Effect::Insert { target, .. }
        | Effect::Prepend { target, .. }
        | Effect::Remove { target, .. }
        | Effect::Move { target, .. }
        | Effect::Focus { target } => target == wanted,
        Effect::Navigate { .. } | Effect::Emit { .. } => false,
    }
}
