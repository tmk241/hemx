#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::marker::PhantomData;
use serde::{Deserialize, Serialize};

pub const SURFACE_SCHEMA_VERSION: u32 = 1;
pub const EFFECT_BATCH_ABI_VERSION: u32 = 1;
/// Media type for the versioned Hemx effect wire format.
pub const HEMX_CONTENT_TYPE: &str = "application/hemx";
pub const RUNTIME_ABI_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BuildFingerprint(pub u64);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AtomSnapshot {
    pub id: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AtomState {
    pub atoms: Vec<AtomSnapshot>,
}

impl AtomState {
    pub fn to_postcard(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }

    pub fn from_postcard(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}

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
    Atom,
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
pub enum NavigateMode {
    Push,
    Replace,
    Redirect,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ScrollBehavior {
    Preserve,
    Top,
    Element(ResourceRef),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Payload {
    Text(String),
    Html(String),
}

impl Payload {
    pub fn text(value: impl ToString) -> Self {
        Self::Text(value.to_string())
    }

    pub fn html(value: SafeHtml) -> Self {
        Self::Html(value.into_string())
    }
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
    Put {
        target: ResourceRef,
        payload: Payload,
    },
    Insert {
        target: ResourceRef,
        key: String,
        payload: Payload,
    },
    Prepend {
        target: ResourceRef,
        key: String,
        payload: Payload,
    },
    Remove {
        target: ResourceRef,
        key: Option<String>,
    },
    Move {
        target: ResourceRef,
        key: String,
        before: Option<String>,
    },
    Focus {
        target: ResourceRef,
    },
    Navigate {
        url: String,
        mode: NavigateMode,
        scroll: ScrollBehavior,
        title: Option<String>,
    },
    Emit {
        name: String,
        payload: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectBatch {
    pub abi_version: u32,
    pub fingerprint: BuildFingerprint,
    pub ops: Vec<Effect>,
}

impl EffectBatch {
    /// The versioned hemx codec is the sole public `EffectBatch` wire API.
    ///
    /// ```compile_fail
    /// let batch = hemx_core::EffectBatch::new(hemx_core::BuildFingerprint(1));
    /// let _ = batch.to_postcard();
    /// ```
    /// Return the exact number of bytes produced by [`Self::to_wire`].
    pub fn encoded_len(&self) -> usize {
        batch_wire_len(self)
    }

    pub fn to_wire(&self) -> Vec<u8> {
        let encoded_len = self.encoded_len();
        let mut out = Vec::with_capacity(encoded_len);
        let initial_capacity = out.capacity();
        write_batch(self, &mut out);
        debug_assert_eq!(out.len(), encoded_len);
        debug_assert_eq!(out.capacity(), initial_capacity);
        out
    }

    pub fn from_wire(bytes: &[u8]) -> Result<Self, WireError> {
        read_batch(bytes)
    }

    pub const fn is_compatible(&self) -> bool {
        self.abi_version == EFFECT_BATCH_ABI_VERSION
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireError {
    BadMagic,
    Truncated,
    InvalidUtf8,
    UnknownTag,
    TrailingBytes,
}

const WIRE_MAGIC: &[u8; 4] = b"HEMX";
const WIRE_BATCH_HEADER_LEN: usize = WIRE_MAGIC.len() + 4 + 8 + 4;

fn batch_wire_len(batch: &EffectBatch) -> usize {
    WIRE_BATCH_HEADER_LEN + batch.ops.iter().map(effect_wire_len).sum::<usize>()
}

fn effect_wire_len(effect: &Effect) -> usize {
    1 + match effect {
        Effect::Put { target, payload } => ref_wire_len(target) + payload_wire_len(payload),
        Effect::Insert {
            target,
            key,
            payload,
        }
        | Effect::Prepend {
            target,
            key,
            payload,
        } => ref_wire_len(target) + str_wire_len(key) + payload_wire_len(payload),
        Effect::Remove { target, key } => {
            ref_wire_len(target) + option_str_wire_len(key.as_deref())
        }
        Effect::Move {
            target,
            key,
            before,
        } => ref_wire_len(target) + str_wire_len(key) + option_str_wire_len(before.as_deref()),
        Effect::Focus { target } => ref_wire_len(target),
        Effect::Navigate {
            url, scroll, title, ..
        } => {
            str_wire_len(url) + 1 + scroll_wire_len(scroll) + option_str_wire_len(title.as_deref())
        }
        Effect::Emit { name, payload } => str_wire_len(name) + str_wire_len(payload),
    }
}

fn ref_wire_len(reference: &ResourceRef) -> usize {
    1 + 4
        + 1
        + match &reference.scope {
            None => 0,
            Some(ScopeKey::KeyValue(value) | ScopeKey::Field(value)) => str_wire_len(value),
        }
}

fn payload_wire_len(payload: &Payload) -> usize {
    1 + match payload {
        Payload::Text(value) | Payload::Html(value) => str_wire_len(value),
    }
}

fn scroll_wire_len(scroll: &ScrollBehavior) -> usize {
    1 + match scroll {
        ScrollBehavior::Preserve | ScrollBehavior::Top => 0,
        ScrollBehavior::Element(target) => ref_wire_len(target),
    }
}

fn option_str_wire_len(value: Option<&str>) -> usize {
    1 + value.map_or(0, str_wire_len)
}

const fn str_wire_len(value: &str) -> usize {
    4 + value.len()
}

fn write_batch(batch: &EffectBatch, out: &mut Vec<u8>) {
    out.extend_from_slice(WIRE_MAGIC);
    write_u32(batch.abi_version, out);
    write_u64(batch.fingerprint.0, out);
    write_u32(batch.ops.len() as u32, out);
    for op in &batch.ops {
        write_effect(op, out);
    }
}

fn write_effect(effect: &Effect, out: &mut Vec<u8>) {
    match effect {
        Effect::Put { target, payload } => {
            write_u8(0, out);
            write_ref(target, out);
            write_payload(payload, out);
        }
        Effect::Insert {
            target,
            key,
            payload,
        } => {
            write_u8(1, out);
            write_ref(target, out);
            write_str(key, out);
            write_payload(payload, out);
        }
        Effect::Prepend {
            target,
            key,
            payload,
        } => {
            write_u8(2, out);
            write_ref(target, out);
            write_str(key, out);
            write_payload(payload, out);
        }
        Effect::Remove { target, key } => {
            write_u8(3, out);
            write_ref(target, out);
            write_option_str(key.as_deref(), out);
        }
        Effect::Move {
            target,
            key,
            before,
        } => {
            write_u8(4, out);
            write_ref(target, out);
            write_str(key, out);
            write_option_str(before.as_deref(), out);
        }
        Effect::Focus { target } => {
            write_u8(5, out);
            write_ref(target, out);
        }
        Effect::Navigate {
            url,
            mode,
            scroll,
            title,
        } => {
            write_u8(6, out);
            write_str(url, out);
            write_u8(
                match mode {
                    NavigateMode::Push => 0,
                    NavigateMode::Replace => 1,
                    NavigateMode::Redirect => 2,
                },
                out,
            );
            write_scroll(scroll, out);
            write_option_str(title.as_deref(), out);
        }
        Effect::Emit { name, payload } => {
            write_u8(7, out);
            write_str(name, out);
            write_str(payload, out);
        }
    }
}

fn write_ref(reference: &ResourceRef, out: &mut Vec<u8>) {
    write_u8(
        match reference.resource.kind {
            ResourceKind::Slot => 0,
            ResourceKind::Atom => 1,
            ResourceKind::Handle => 2,
            ResourceKind::Form => 3,
        },
        out,
    );
    write_u32(reference.resource.id, out);
    match &reference.scope {
        None => write_u8(0, out),
        Some(ScopeKey::KeyValue(value)) => {
            write_u8(1, out);
            write_str(value, out);
        }
        Some(ScopeKey::Field(value)) => {
            write_u8(2, out);
            write_str(value, out);
        }
    }
}

fn write_payload(payload: &Payload, out: &mut Vec<u8>) {
    match payload {
        Payload::Text(value) => {
            write_u8(0, out);
            write_str(value, out);
        }
        Payload::Html(value) => {
            write_u8(1, out);
            write_str(value, out);
        }
    }
}

fn write_scroll(scroll: &ScrollBehavior, out: &mut Vec<u8>) {
    match scroll {
        ScrollBehavior::Preserve => write_u8(0, out),
        ScrollBehavior::Top => write_u8(1, out),
        ScrollBehavior::Element(target) => {
            write_u8(2, out);
            write_ref(target, out);
        }
    }
}

fn write_option_str(value: Option<&str>, out: &mut Vec<u8>) {
    match value {
        None => write_u8(0, out),
        Some(value) => {
            write_u8(1, out);
            write_str(value, out);
        }
    }
}

fn write_str(value: &str, out: &mut Vec<u8>) {
    write_u32(value.len() as u32, out);
    out.extend_from_slice(value.as_bytes());
}

fn write_u8(value: u8, out: &mut Vec<u8>) {
    out.push(value);
}

fn write_u32(value: u32, out: &mut Vec<u8>) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_u64(value: u64, out: &mut Vec<u8>) {
    out.extend_from_slice(&value.to_le_bytes());
}

struct WireReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> WireReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn finish(&self) -> Result<(), WireError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(WireError::TrailingBytes)
        }
    }

    fn read_u8(&mut self) -> Result<u8, WireError> {
        let bytes = self.read_exact(1)?;
        Ok(bytes[0])
    }

    fn read_u32(&mut self) -> Result<u32, WireError> {
        let bytes = self.read_exact(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_u64(&mut self) -> Result<u64, WireError> {
        let bytes = self.read_exact(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn read_str(&mut self) -> Result<String, WireError> {
        let len = self.read_u32()? as usize;
        let bytes = self.read_exact(len)?;
        core::str::from_utf8(bytes)
            .map(ToString::to_string)
            .map_err(|_| WireError::InvalidUtf8)
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8], WireError> {
        let remaining = &self.bytes[self.offset..];
        let bytes = remaining.get(..len).ok_or(WireError::Truncated)?;
        self.offset += len;
        Ok(bytes)
    }
}

fn read_batch(bytes: &[u8]) -> Result<EffectBatch, WireError> {
    let mut reader = WireReader::new(bytes);
    if reader.read_exact(WIRE_MAGIC.len())? != WIRE_MAGIC {
        return Err(WireError::BadMagic);
    }
    let abi_version = reader.read_u32()?;
    let fingerprint = BuildFingerprint(reader.read_u64()?);
    let ops_len = reader.read_u32()?;
    let mut ops = Vec::new();
    for _ in 0..ops_len {
        ops.push(read_effect(&mut reader)?);
    }
    reader.finish()?;
    Ok(EffectBatch {
        abi_version,
        fingerprint,
        ops,
    })
}

fn read_effect(reader: &mut WireReader<'_>) -> Result<Effect, WireError> {
    match reader.read_u8()? {
        0 => Ok(Effect::Put {
            target: read_ref(reader)?,
            payload: read_payload(reader)?,
        }),
        1 => Ok(Effect::Insert {
            target: read_ref(reader)?,
            key: reader.read_str()?,
            payload: read_payload(reader)?,
        }),
        2 => Ok(Effect::Prepend {
            target: read_ref(reader)?,
            key: reader.read_str()?,
            payload: read_payload(reader)?,
        }),
        3 => Ok(Effect::Remove {
            target: read_ref(reader)?,
            key: read_option_str(reader)?,
        }),
        4 => Ok(Effect::Move {
            target: read_ref(reader)?,
            key: reader.read_str()?,
            before: read_option_str(reader)?,
        }),
        5 => Ok(Effect::Focus {
            target: read_ref(reader)?,
        }),
        6 => Ok(Effect::Navigate {
            url: reader.read_str()?,
            mode: match reader.read_u8()? {
                0 => NavigateMode::Push,
                1 => NavigateMode::Replace,
                2 => NavigateMode::Redirect,
                _ => return Err(WireError::UnknownTag),
            },
            scroll: read_scroll(reader)?,
            title: read_option_str(reader)?,
        }),
        7 => Ok(Effect::Emit {
            name: reader.read_str()?,
            payload: reader.read_str()?,
        }),
        _ => Err(WireError::UnknownTag),
    }
}

fn read_ref(reader: &mut WireReader<'_>) -> Result<ResourceRef, WireError> {
    let kind = match reader.read_u8()? {
        0 => ResourceKind::Slot,
        1 => ResourceKind::Atom,
        2 => ResourceKind::Handle,
        3 => ResourceKind::Form,
        _ => return Err(WireError::UnknownTag),
    };
    let resource = ResourceId::new(kind, reader.read_u32()?);
    let scope = match reader.read_u8()? {
        0 => None,
        1 => Some(ScopeKey::KeyValue(reader.read_str()?)),
        2 => Some(ScopeKey::Field(reader.read_str()?)),
        _ => return Err(WireError::UnknownTag),
    };
    Ok(ResourceRef { resource, scope })
}

fn read_payload(reader: &mut WireReader<'_>) -> Result<Payload, WireError> {
    match reader.read_u8()? {
        0 => Ok(Payload::Text(reader.read_str()?)),
        1 => Ok(Payload::Html(reader.read_str()?)),
        _ => Err(WireError::UnknownTag),
    }
}

fn read_scroll(reader: &mut WireReader<'_>) -> Result<ScrollBehavior, WireError> {
    match reader.read_u8()? {
        0 => Ok(ScrollBehavior::Preserve),
        1 => Ok(ScrollBehavior::Top),
        2 => Ok(ScrollBehavior::Element(read_ref(reader)?)),
        _ => Err(WireError::UnknownTag),
    }
}

fn read_option_str(reader: &mut WireReader<'_>) -> Result<Option<String>, WireError> {
    match reader.read_u8()? {
        0 => Ok(None),
        1 => Ok(Some(reader.read_str()?)),
        _ => Err(WireError::UnknownTag),
    }
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

    pub fn text(self, value: impl ToString) -> Effect {
        Effect::Put {
            target: ResourceRef::unscoped(self.id),
            payload: Payload::text(value),
        }
    }

    pub fn html(self, value: impl Into<SafeHtml>) -> Effect {
        Effect::Put {
            target: ResourceRef::unscoped(self.id),
            payload: Payload::html(value.into()),
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

    pub fn append_text(self, key: K, value: T) -> Effect
    where
        T: ToString,
    {
        Effect::Insert {
            target: ResourceRef::unscoped(self.id),
            key: key.to_string(),
            payload: Payload::text(value),
        }
    }

    pub fn prepend_text(self, key: K, value: T) -> Effect
    where
        T: ToString,
    {
        Effect::Prepend {
            target: ResourceRef::unscoped(self.id),
            key: key.to_string(),
            payload: Payload::text(value),
        }
    }

    pub fn replace_text(self, key: K, value: T) -> Effect
    where
        T: ToString,
    {
        let key = key.to_string();
        Effect::Put {
            target: ResourceRef {
                resource: self.id,
                scope: Some(ScopeKey::KeyValue(key)),
            },
            payload: Payload::text(value),
        }
    }

    pub fn append_html(self, key: K, value: impl Into<SafeHtml>) -> Effect {
        Effect::Insert {
            target: ResourceRef::unscoped(self.id),
            key: key.to_string(),
            payload: Payload::html(value.into()),
        }
    }

    pub fn prepend_html(self, key: K, value: impl Into<SafeHtml>) -> Effect {
        Effect::Prepend {
            target: ResourceRef::unscoped(self.id),
            key: key.to_string(),
            payload: Payload::html(value.into()),
        }
    }

    pub fn replace_html(self, key: K, value: impl Into<SafeHtml>) -> Effect {
        let key = key.to_string();
        Effect::Put {
            target: ResourceRef {
                resource: self.id,
                scope: Some(ScopeKey::KeyValue(key)),
            },
            payload: Payload::html(value.into()),
        }
    }

    pub fn remove(self, key: K) -> Effect {
        Effect::Remove {
            target: ResourceRef::unscoped(self.id),
            key: Some(key.to_string()),
        }
    }

    pub fn move_before(self, key: K, before: K) -> Effect {
        Effect::Move {
            target: ResourceRef::unscoped(self.id),
            key: key.to_string(),
            before: Some(before.to_string()),
        }
    }

    pub fn move_to_end(self, key: K) -> Effect {
        Effect::Move {
            target: ResourceRef::unscoped(self.id),
            key: key.to_string(),
            before: None,
        }
    }
}

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct Atom<T> {
    id: ResourceId,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for Atom<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Atom<T> {}

impl<T> core::fmt::Display for Atom<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.id.id.fmt(f)
    }
}

impl<T> Atom<T> {
    pub const fn new(id: u32) -> Self {
        Self {
            id: ResourceId::new(ResourceKind::Atom, id),
            _marker: PhantomData,
        }
    }

    pub const fn id(self) -> ResourceId {
        self.id
    }

    pub fn set(self, value: impl ToString) -> Effect {
        Effect::Put {
            target: ResourceRef::unscoped(self.id),
            payload: Payload::text(value),
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

impl std::error::Error for FormError {}

pub trait FormValue: Sized {
    fn parse_form_value(value: &str) -> Result<Self, String>;
}

impl<T> FormValue for T
where
    T: std::str::FromStr,
{
    fn parse_form_value(value: &str) -> Result<Self, String> {
        value.parse().map_err(|_| "invalid form value".to_owned())
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

    pub fn reset(self) -> Effect {
        Effect::Emit {
            name: String::from("hemx:form-reset"),
            payload: self.id.id.to_string(),
        }
    }

    pub fn clear(self) -> Effect {
        self.reset()
    }

    pub fn clear_field(self, field: impl Into<String>) -> Effect {
        Effect::Put {
            target: self.field(field),
            payload: Payload::text(""),
        }
    }

    pub fn error(self, field: impl Into<String>, message: impl ToString) -> Effect {
        let field = field.into();
        let message = message.to_string();
        let mut payload = self.id.id.to_string();
        payload.push('\u{1f}');
        payload.push_str(&field);
        payload.push('\u{1f}');
        payload.push_str(&message);
        Effect::Emit {
            name: String::from("hemx:form-error"),
            payload,
        }
    }

    pub fn focus(self, field: impl Into<String>) -> Effect {
        Effect::Focus {
            target: self.field(field),
        }
    }

    pub fn disable_while_pending(self) -> Effect {
        Effect::Emit {
            name: String::from("hemx:form-disable-while-pending"),
            payload: self.id.id.to_string(),
        }
    }
}

pub fn navigate(url: impl Into<String>) -> Effect {
    push(url)
}

pub fn push(url: impl Into<String>) -> Effect {
    Effect::Navigate {
        url: url.into(),
        mode: NavigateMode::Push,
        scroll: ScrollBehavior::Top,
        title: None,
    }
}

pub fn replace(url: impl Into<String>) -> Effect {
    Effect::Navigate {
        url: url.into(),
        mode: NavigateMode::Replace,
        scroll: ScrollBehavior::Top,
        title: None,
    }
}

pub fn redirect(url: impl Into<String>) -> Effect {
    Effect::Navigate {
        url: url.into(),
        mode: NavigateMode::Redirect,
        scroll: ScrollBehavior::Top,
        title: None,
    }
}

pub fn event(name: impl Into<String>, payload: impl Into<String>) -> Effect {
    Effect::Emit {
        name: name.into(),
        payload: payload.into(),
    }
}
