#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::marker::PhantomData;
use serde::{Deserialize, Serialize};

pub const SURFACE_SCHEMA_VERSION: u32 = 2;
pub const EFFECT_BATCH_ABI_VERSION: u32 = 3;
pub const RUNTIME_ABI_VERSION: u32 = 3;

mod fold;
mod rows;
mod wire;
pub use fold::{fold_batches, fold_run, folds_into};
pub use rows::{order_hash, row_steps, RowStep};
pub use wire::{WireError, MAX_BATCH_BYTES, MAX_EFFECTS, MAX_FIELD_BYTES, MAX_ROW_KEYS};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BuildFingerprint(pub u64);

impl BuildFingerprint {
    pub const fn from_parts(parts: &[u32]) -> Self {
        let mut hash = 0xcbf29ce484222325u64;
        let mut i = 0;
        while i < parts.len() {
            hash ^= parts[i] as u64;
            hash = hash.wrapping_mul(0x100000001b3);
            i += 1;
        }
        Self(hash)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ResourceKind {
    Slot,
    Handle,
    Form,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ResourceId {
    pub kind: ResourceKind,
    pub id: u32,
}

impl ResourceId {
    pub const fn new(kind: ResourceKind, id: u32) -> Self {
        Self { kind, id }
    }
}

/// A generated UI target that can be inspected without exposing raw slots.
pub trait GeneratedTarget {
    #[doc(hidden)]
    fn __hemx_resource_id(self) -> ResourceId;
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ScopeKey {
    KeyValue(String),
    Field(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ResourceRef {
    pub resource: ResourceId,
    pub scope: Option<ScopeKey>,
}

impl ResourceRef {
    pub const fn unscoped(resource: ResourceId) -> Self {
        Self {
            resource,
            scope: None,
        }
    }

    pub fn scoped(resource: ResourceId, scope: ScopeKey) -> Self {
        Self {
            resource,
            scope: Some(scope),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum PatchMode {
    Morph,
    Replace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ScrollBlock {
    Start,
    Center,
    End,
    Nearest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ScrollBehavior {
    Auto,
    Instant,
    Smooth,
}

pub type Url = String;
pub type EventRef = String;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum HistoryMode {
    Push,
    Replace,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SafeHtml(String);

impl SafeHtml {
    pub fn trusted(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn into_string(self) -> String {
        self.0
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn join(fragments: impl IntoIterator<Item = SafeHtml>) -> Self {
        fragments.into_iter().collect()
    }
}

impl FromIterator<SafeHtml> for SafeHtml {
    fn from_iter<T: IntoIterator<Item = SafeHtml>>(iter: T) -> Self {
        let mut html = String::new();
        for fragment in iter {
            html.push_str(fragment.as_str());
        }
        Self(html)
    }
}

impl AsRef<str> for SafeHtml {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl core::fmt::Display for SafeHtml {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A generated, checked CSS class token.
///
/// Plain CSS/SCSS owns appearance; hemx only gives Rust a typed reference to
/// class names discovered from build inputs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct CssClass {
    name: &'static str,
}

impl CssClass {
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    pub const fn as_str(self) -> &'static str {
        self.name
    }

    pub fn with(self, class: CssClass) -> CssClasses {
        CssClasses::from([self, class])
    }

    pub fn with_if(self, condition: bool, class: CssClass) -> CssClasses {
        CssClasses::from(self).with_if(condition, class)
    }
}

impl AsRef<str> for CssClass {
    fn as_ref(&self) -> &str {
        self.name
    }
}

impl core::fmt::Display for CssClass {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name)
    }
}

/// A generated, checked parameter token.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ParamName {
    name: &'static str,
}

impl ParamName {
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    pub const fn as_str(self) -> &'static str {
        self.name
    }
}

impl AsRef<str> for ParamName {
    fn as_ref(&self) -> &str {
        self.name
    }
}

impl core::fmt::Display for ParamName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name)
    }
}

/// A generated, checked component token.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ComponentRef {
    name: &'static str,
}

impl ComponentRef {
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    pub const fn as_str(self) -> &'static str {
        self.name
    }
}

impl AsRef<str> for ComponentRef {
    fn as_ref(&self) -> &str {
        self.name
    }
}

impl core::fmt::Display for ComponentRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name)
    }
}

/// A generated, checked event token.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct EventName {
    name: &'static str,
}

impl EventName {
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    pub const fn as_str(self) -> &'static str {
        self.name
    }

    pub fn emit(self, payload: impl Into<String>) -> Effect {
        event(self, payload)
    }
}

impl AsRef<str> for EventName {
    fn as_ref(&self) -> &str {
        self.name
    }
}

impl From<EventName> for String {
    fn from(event: EventName) -> Self {
        event.name.to_string()
    }
}

impl core::fmt::Display for EventName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name)
    }
}

/// A small displayable list of generated CSS class tokens for hemplate `+class`.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct CssClasses {
    names: String,
}

impl CssClasses {
    pub fn new(classes: impl IntoIterator<Item = CssClass>) -> Self {
        let mut names = String::new();
        for class in classes {
            if !names.is_empty() {
                names.push(' ');
            }
            names.push_str(class.as_str());
        }
        Self { names }
    }

    pub fn as_str(&self) -> &str {
        &self.names
    }

    pub fn with(mut self, class: CssClass) -> Self {
        if !self.names.is_empty() {
            self.names.push(' ');
        }
        self.names.push_str(class.as_str());
        self
    }

    pub fn with_if(self, condition: bool, class: CssClass) -> Self {
        if condition {
            self.with(class)
        } else {
            self
        }
    }
}

impl From<CssClass> for CssClasses {
    fn from(class: CssClass) -> Self {
        Self::new([class])
    }
}

impl<const N: usize> From<[CssClass; N]> for CssClasses {
    fn from(classes: [CssClass; N]) -> Self {
        Self::new(classes)
    }
}

impl AsRef<str> for CssClasses {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl core::fmt::Display for CssClasses {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.names)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct FormContract {
    pub fields: &'static [FormField],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct FormField {
    pub name: &'static str,
    pub kind: FormControlKind,
    pub required: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FormControlKind {
    Text,
    Number {
        min: Option<&'static str>,
        max: Option<&'static str>,
        step: Option<&'static str>,
    },
    Checkbox,
    Radio,
    Select {
        multiple: bool,
    },
    TextArea,
    File,
    Hidden,
    Submit,
    Other {
        tag: &'static str,
        input_type: Option<&'static str>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    Patch {
        target: ResourceRef,
        mode: PatchMode,
        html: SafeHtml,
    },
    /// Declare a collection's complete state: exactly `order`, in that order,
    /// with `rows` supplying the elements of new or changed rows. Keys absent
    /// from `order` are removed; ordered keys without a row keep their element.
    Rows {
        /// The existing collection destination, optionally scoped to one instance.
        target: ResourceRef,
        order: Vec<String>,
        rows: Vec<(String, SafeHtml)>,
    },
    /// Change a collection from the order named `from` to the order named `to`
    /// (see [`order_hash`]) by applying `steps` in sequence. `rows` supplies the
    /// elements of created rows and new content for live ones. A client applies
    /// it only at `from`, skips it at `to`, and rejects it anywhere else, so it
    /// is as idempotent as [`Effect::Rows`] while carrying only what changed.
    RowsDelta {
        target: ResourceRef,
        from: u64,
        to: u64,
        steps: Vec<RowStep>,
        rows: Vec<(String, SafeHtml)>,
    },
    Focus {
        target: ResourceRef,
        prevent_scroll: bool,
    },
    Scroll {
        target: ResourceRef,
        block: ScrollBlock,
        behavior: ScrollBehavior,
    },
    Visit {
        url: Url,
        history: HistoryMode,
    },
    Dispatch {
        event: EventRef,
        payload: Vec<u8>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectBatch {
    pub abi_version: u32,
    pub fingerprint: BuildFingerprint,
    pub ops: Vec<Effect>,
}

pub trait IntoEffect {
    fn append_to(self, ops: &mut Vec<Effect>);

    fn into_batch(self, fingerprint: BuildFingerprint) -> EffectBatch
    where
        Self: Sized,
    {
        let mut ops = Vec::new();
        self.append_to(&mut ops);
        EffectBatch {
            abi_version: EFFECT_BATCH_ABI_VERSION,
            fingerprint,
            ops,
        }
    }
}

impl IntoEffect for Effect {
    fn append_to(self, ops: &mut Vec<Effect>) {
        ops.push(self);
    }
}

impl IntoEffect for () {
    fn append_to(self, _ops: &mut Vec<Effect>) {}
}

impl<T: IntoEffect> IntoEffect for Option<T> {
    fn append_to(self, ops: &mut Vec<Effect>) {
        if let Some(effect) = self {
            effect.append_to(ops);
        }
    }
}

impl<T: IntoEffect> IntoEffect for Vec<T> {
    fn append_to(self, ops: &mut Vec<Effect>) {
        for effect in self {
            effect.append_to(ops);
        }
    }
}

impl<T: IntoEffect, const N: usize> IntoEffect for [T; N] {
    fn append_to(self, ops: &mut Vec<Effect>) {
        for effect in self {
            effect.append_to(ops);
        }
    }
}

macro_rules! impl_tuple_into_effect {
    ($($name:ident $idx:tt),+) => {
        impl<$($name),+> IntoEffect for ($($name,)+)
        where
            $($name: IntoEffect),+
        {
            fn append_to(self, ops: &mut Vec<Effect>) {
                $(self.$idx.append_to(ops);)+
            }
        }
    };
}

impl_tuple_into_effect!(A 0, B 1);
impl_tuple_into_effect!(A 0, B 1, C 2);
impl_tuple_into_effect!(A 0, B 1, C 2, D 3);
impl_tuple_into_effect!(A 0, B 1, C 2, D 3, E 4);
impl_tuple_into_effect!(A 0, B 1, C 2, D 3, E 4, F 5);
impl_tuple_into_effect!(A 0, B 1, C 2, D 3, E 4, F 5, G 6);
impl_tuple_into_effect!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7);
impl_tuple_into_effect!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8);
impl_tuple_into_effect!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9);
impl_tuple_into_effect!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10);
impl_tuple_into_effect!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11);

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct Slot<T> {
    id: ResourceId,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for Slot<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Slot<T> {}

impl<T> core::fmt::Display for Slot<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.id.id.fmt(f)
    }
}

impl<T> Slot<T> {
    pub const fn new(id: u32) -> Self {
        Self {
            id: ResourceId::new(ResourceKind::Slot, id),
            _marker: PhantomData,
        }
    }

    pub const fn id(self) -> ResourceId {
        self.id
    }

    /// Declare this collection's complete key order plus the elements of rows
    /// that are new or changed. Keys left out of `order` are removed.
    pub fn rows_html<O, K, H>(
        self,
        order: impl IntoIterator<Item = O>,
        rows: impl IntoIterator<Item = (K, H)>,
    ) -> Effect
    where
        O: ToString,
        K: ToString,
        H: Into<SafeHtml>,
    {
        Effect::Rows {
            target: ResourceRef::unscoped(self.id),
            order: order.into_iter().map(|key| key.to_string()).collect(),
            rows: rows
                .into_iter()
                .map(|(key, html)| (key.to_string(), html.into()))
                .collect(),
        }
    }

    /// Change this collection from the key order `before` to `after`, supplying
    /// the elements of new rows and the new content of changed ones. Only the
    /// steps between the two orders travel; see [`Effect::RowsDelta`].
    pub fn rows_delta_html<B, A, K, H>(
        self,
        before: impl IntoIterator<Item = B>,
        after: impl IntoIterator<Item = A>,
        rows: impl IntoIterator<Item = (K, H)>,
    ) -> Effect
    where
        B: ToString,
        A: ToString,
        K: ToString,
        H: Into<SafeHtml>,
    {
        let before: Vec<String> = before.into_iter().map(|key| key.to_string()).collect();
        let after: Vec<String> = after.into_iter().map(|key| key.to_string()).collect();
        Effect::RowsDelta {
            target: ResourceRef::unscoped(self.id),
            from: order_hash(&before),
            to: order_hash(&after),
            steps: row_steps(&before, &after),
            rows: rows
                .into_iter()
                .map(|(key, html)| (key.to_string(), html.into()))
                .collect(),
        }
    }

    /// Patch this slot with escaped text, not interpreted markup.
    pub fn text(self, value: impl ToString) -> Effect {
        let text = value.to_string();
        let mut escaped = String::with_capacity(text.len());
        for character in text.chars() {
            match character {
                '&' => escaped.push_str("&amp;"),
                '<' => escaped.push_str("&lt;"),
                '>' => escaped.push_str("&gt;"),
                '"' => escaped.push_str("&quot;"),
                '\'' => escaped.push_str("&#39;"),
                _ => escaped.push(character),
            }
        }
        self.html(SafeHtml::trusted(escaped))
    }

    pub fn html(self, value: impl Into<SafeHtml>) -> Effect {
        Effect::Patch {
            target: ResourceRef::unscoped(self.id),
            mode: PatchMode::Morph,
            html: value.into(),
        }
    }
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct KeyedSlot<K, T> {
    id: ResourceId,
    _marker: PhantomData<fn(K) -> T>,
}

impl<K, T> Clone for KeyedSlot<K, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K, T> Copy for KeyedSlot<K, T> {}

impl<K, T> core::fmt::Display for KeyedSlot<K, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.id.id.fmt(f)
    }
}

impl<K, T> KeyedSlot<K, T>
where
    K: ToString,
{
    pub const fn new(id: u32) -> Self {
        Self {
            id: ResourceId::new(ResourceKind::Slot, id),
            _marker: PhantomData,
        }
    }

    pub const fn id(self) -> ResourceId {
        self.id
    }

    pub fn replace_text(self, key: K, value: T) -> Effect
    where
        T: ToString,
    {
        let key = key.to_string();
        Effect::Patch {
            target: ResourceRef {
                resource: self.id,
                scope: Some(ScopeKey::KeyValue(key)),
            },
            mode: PatchMode::Morph,
            html: SafeHtml::trusted(value.to_string()),
        }
    }

    pub fn replace_html(self, key: K, value: impl Into<SafeHtml>) -> Effect {
        let key = key.to_string();
        Effect::Patch {
            target: ResourceRef {
                resource: self.id,
                scope: Some(ScopeKey::KeyValue(key)),
            },
            mode: PatchMode::Morph,
            html: value.into(),
        }
    }
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct Handle<I> {
    id: ResourceId,
    _marker: PhantomData<fn(I)>,
}

impl<I> Clone for Handle<I> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<I> Copy for Handle<I> {}

impl<I> Handle<I> {
    pub const fn new(id: u32) -> Self {
        Self {
            id: ResourceId::new(ResourceKind::Handle, id),
            _marker: PhantomData,
        }
    }

    pub const fn id(self) -> ResourceId {
        self.id
    }
}

impl<I> core::fmt::Display for Handle<I> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.id.id.fmt(f)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormError {
    message: String,
}

impl FormError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl core::fmt::Display for FormError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl core::error::Error for FormError {}

pub trait FormValue: Sized {
    fn parse_form_value(value: &str) -> Result<Self, String>;
}

impl<T> FormValue for T
where
    T: core::str::FromStr,
{
    fn parse_form_value(value: &str) -> Result<Self, String> {
        value.parse().map_err(|_| "invalid form value".to_string())
    }
}

pub trait FromForm: Sized {
    fn from_form_fields(fields: &[(String, String)]) -> Result<Self, FormError>;
}

pub trait FormModel {}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct Form<T> {
    id: ResourceId,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for Form<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Form<T> {}

impl<T> core::fmt::Display for Form<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.id.id.fmt(f)
    }
}

impl<T> GeneratedTarget for Form<T> {
    fn __hemx_resource_id(self) -> ResourceId {
        self.id
    }
}

impl<T> Form<T> {
    pub const fn new(id: u32) -> Self {
        Self {
            id: ResourceId::new(ResourceKind::Form, id),
            _marker: PhantomData,
        }
    }

    pub const fn id(self) -> ResourceId {
        self.id
    }

    pub const fn typed<U>(self) -> Form<U> {
        Form {
            id: self.id,
            _marker: PhantomData,
        }
    }

    pub fn field(self, name: impl Into<String>) -> ResourceRef {
        ResourceRef::scoped(self.id, ScopeKey::Field(name.into()))
    }

    pub fn clear_field(self, field: impl Into<String>) -> Effect {
        Effect::Patch {
            target: self.field(field),
            mode: PatchMode::Morph,
            html: SafeHtml::trusted(""),
        }
    }

    pub fn focus(self, field: impl Into<String>) -> Effect {
        Effect::Focus {
            target: self.field(field),
            prevent_scroll: false,
        }
    }
}

pub fn navigate(url: impl Into<String>) -> Effect {
    push(url)
}

pub fn push(url: impl Into<String>) -> Effect {
    Effect::Visit {
        url: url.into(),
        history: HistoryMode::Push,
    }
}

pub fn replace(url: impl Into<String>) -> Effect {
    Effect::Visit {
        url: url.into(),
        history: HistoryMode::Replace,
    }
}

pub fn redirect(url: impl Into<String>) -> Effect {
    Effect::Visit {
        url: url.into(),
        history: HistoryMode::Replace,
    }
}

pub fn event(name: impl Into<String>, payload: impl Into<String>) -> Effect {
    Effect::Dispatch {
        event: name.into(),
        payload: payload.into().into_bytes(),
    }
}
