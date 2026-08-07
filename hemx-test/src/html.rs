use hemx_core::{GeneratedTarget, Handle, ResourceKind};
use scraper::{Html, Selector};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

/// An owned, parsed HTML document or fragment.
///
/// The underlying parser is intentionally private so application tests do not
/// become coupled to `scraper`'s public types.
pub struct HtmlInspector {
    source: Arc<str>,
    origin: Arc<str>,
    parsed: Html,
}

impl fmt::Debug for HtmlInspector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HtmlInspector")
            .field("origin", &self.origin)
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl HtmlInspector {
    pub(crate) fn document(source: String, origin: String) -> Self {
        Self {
            parsed: Html::parse_document(&source),
            source: source.into(),
            origin: origin.into(),
        }
    }

    pub(crate) fn fragment(source: String, origin: String) -> Self {
        Self {
            parsed: Html::parse_fragment(&source),
            source: source.into(),
            origin: origin.into(),
        }
    }

    /// Return the original HTML supplied to the inspector.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Describe where the inspected HTML came from.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Select elements with a CSS selector and copy their observable structure
    /// into an owned result.
    pub fn select(&self, selector: &str) -> Result<HtmlSelection, HtmlInspectionError> {
        let parsed_selector = Selector::parse(selector).map_err(|error| {
            HtmlInspectionError::new(format!(
                "invalid CSS selector {selector:?} while inspecting {}: {error:?}",
                self.origin
            ))
        })?;
        let elements = self
            .parsed
            .select(&parsed_selector)
            .map(|element| HtmlElement {
                name: element.value().name().to_owned(),
                text: normalize_text(element.text()),
                attributes: element
                    .value()
                    .attrs()
                    .map(|(name, value)| (name.to_owned(), value.to_owned()))
                    .collect(),
                html: element.html(),
            })
            .collect();

        Ok(HtmlSelection {
            selector: selector.to_owned(),
            origin: Arc::clone(&self.origin),
            source: Arc::clone(&self.source),
            elements,
        })
    }

    /// Select elements carrying the runtime marker for a generated target.
    pub fn select_target(&self, target: impl GeneratedTarget) -> HtmlSelection {
        let resource = target.__hemx_resource_id();
        self.selection_or_panic(&attribute_selector(
            resource_attribute(resource.kind),
            &resource.id.to_string(),
        ))
    }

    /// Assert that rendered HTML contains a generated target marker.
    #[track_caller]
    pub fn assert_target(&self, target: impl GeneratedTarget) {
        self.select_target(target).assert_exists();
    }

    /// Select elements carrying the runtime marker for a typed handle.
    pub fn select_handle<I>(&self, handle: Handle<I>) -> HtmlSelection {
        self.selection_or_panic(&attribute_selector("data-hid", &handle.to_string()))
    }

    /// Assert that rendered HTML contains a typed handle marker.
    #[track_caller]
    pub fn assert_handle<I>(&self, handle: Handle<I>) {
        self.select_handle(handle).assert_exists();
    }

    /// Assert that at least one element matches a CSS selector.
    #[track_caller]
    pub fn assert_exists(&self, selector: &str) {
        self.selection_or_panic(selector).assert_exists();
    }

    /// Assert that exactly `expected` elements match a CSS selector.
    #[track_caller]
    pub fn assert_count(&self, selector: &str, expected: usize) {
        self.selection_or_panic(selector).assert_count(expected);
    }

    /// Assert that one element matches a selector and has the expected
    /// whitespace-normalized text.
    #[track_caller]
    pub fn assert_text(&self, selector: &str, expected: &str) {
        self.selection_or_panic(selector).assert_text(expected);
    }

    /// Assert that one element matches a selector and has an exact attribute
    /// value.
    #[track_caller]
    pub fn assert_attribute(&self, selector: &str, name: &str, expected: &str) {
        self.selection_or_panic(selector)
            .assert_attribute(name, expected);
    }

    #[track_caller]
    fn selection_or_panic(&self, selector: &str) -> HtmlSelection {
        self.select(selector)
            .unwrap_or_else(|error| panic!("{error}"))
    }
}

/// An owned set of elements selected from an [`HtmlInspector`].
#[derive(Clone, Debug)]
pub struct HtmlSelection {
    selector: String,
    origin: Arc<str>,
    source: Arc<str>,
    elements: Vec<HtmlElement>,
}

impl HtmlSelection {
    pub fn selector(&self) -> &str {
        &self.selector
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub fn elements(&self) -> &[HtmlElement] {
        &self.elements
    }

    /// Assert that this selection contains at least one element.
    #[track_caller]
    pub fn assert_exists(&self) {
        assert!(
            !self.is_empty(),
            "expected at least one element matching {:?} in {}; found none. source: {}",
            self.selector,
            self.origin,
            excerpt(&self.source)
        );
    }

    /// Assert that this selection contains exactly `expected` elements.
    #[track_caller]
    pub fn assert_count(&self, expected: usize) {
        assert_eq!(
            self.len(),
            expected,
            "unexpected match count for selector {:?} in {}. matches: {}. source: {}",
            self.selector,
            self.origin,
            matching_markup(&self.elements),
            excerpt(&self.source)
        );
    }

    /// Assert that this selection has one element with exact
    /// whitespace-normalized text.
    #[track_caller]
    pub fn assert_text(&self, expected: &str) {
        let element = self.only_element("text");
        assert_eq!(
            element.text,
            expected,
            "unexpected text for selector {:?} in {}. element: {}",
            self.selector,
            self.origin,
            excerpt(&element.html)
        );
    }

    /// Assert that this selection has one element with an exact attribute
    /// value.
    #[track_caller]
    pub fn assert_attribute(&self, name: &str, expected: &str) {
        let element = self.only_element("an attribute");
        let actual = element.attribute(name);
        assert_eq!(
            actual,
            Some(expected),
            "unexpected attribute {name:?} for selector {:?} in {}. element: {}",
            self.selector,
            self.origin,
            excerpt(&element.html)
        );
    }

    #[track_caller]
    fn only_element(&self, assertion: &str) -> &HtmlElement {
        assert_eq!(
            self.elements.len(),
            1,
            "expected exactly one element matching {:?} in {} before asserting {assertion}; found {}. matches: {}. source: {}",
            self.selector,
            self.origin,
            self.elements.len(),
            matching_markup(&self.elements),
            excerpt(&self.source)
        );
        &self.elements[0]
    }
}

/// Owned observable structure for one selected HTML element.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlElement {
    name: String,
    text: String,
    attributes: BTreeMap<String, String>,
    html: String,
}

impl HtmlElement {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return whitespace-normalized descendant text.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(String::as_str)
    }

    pub fn attributes(&self) -> impl Iterator<Item = (&str, &str)> {
        self.attributes
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    pub fn html(&self) -> &str {
        &self.html
    }
}

/// A structural HTML inspection error with an owned diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlInspectionError {
    message: String,
}

impl HtmlInspectionError {
    pub(crate) fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for HtmlInspectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for HtmlInspectionError {}

fn resource_attribute(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Slot => "data-sid",
        ResourceKind::Atom => "data-aid",
        ResourceKind::Handle => "data-hid",
        ResourceKind::Form => "data-fid",
    }
}

fn attribute_selector(name: &str, value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!(r#"[{name}="{escaped}"]"#)
}

fn normalize_text<'a>(text: impl Iterator<Item = &'a str>) -> String {
    text.flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
}

fn matching_markup(elements: &[HtmlElement]) -> String {
    if elements.is_empty() {
        return String::from("none");
    }
    elements
        .iter()
        .take(4)
        .map(|element| excerpt(&element.html))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn excerpt(value: &str) -> String {
    const LIMIT: usize = 500;
    if value.chars().count() <= LIMIT {
        return value.to_owned();
    }
    let mut excerpt = value.chars().take(LIMIT).collect::<String>();
    excerpt.push('…');
    excerpt
}
