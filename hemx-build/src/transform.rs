//! Compile-time transforms: authored Hemx markers become final runtime markers.
//!
//! Every transform input is a static template fact, so the build script hands
//! Hemplate one transform per template instead of rewriting rendered HTML.
//! Interpolated text and trusted raw HTML therefore reach the output unchanged.

use super::*;
use crate::artifact::{Resources, SlotRole};
use hemplate_core::surface::SurfaceNode;
use hemplate_core::transform::{source_hash, Edit, Transform};

/// Static edits that turn one template's authored markers into runtime markers.
pub(super) fn template_transform(
    resources: &Resources,
    fingerprint: u64,
    source: &str,
    surface: &SurfaceDocument,
) -> Transform {
    let mut edits = Vec::new();
    for node in &surface.nodes {
        let SurfaceNodeKind::Element { tag } = &node.kind else {
            continue;
        };
        // A `<template>` renders only its content, never its own attributes.
        let rendered_attrs = if tag == "template" {
            &[][..]
        } else {
            &node.attrs[..]
        };
        for attr in rendered_attrs
            .iter()
            .filter(|attr| attr.origin == AttributeOrigin::Static)
        {
            if let Some(text) = lowered_attribute(resources, source, attr) {
                edits.push(Edit {
                    range: attr.span.clone(),
                    text,
                });
            }
        }
        if tag == "form" {
            if let Some(edit) = action_fields(resources, fingerprint, surface, node) {
                edits.push(edit);
            }
        }
    }
    edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
    Transform {
        source_hash: source_hash(source),
        edits,
    }
}

fn lowered_attribute(
    resources: &Resources,
    source: &str,
    attr: &SurfaceAttribute,
) -> Option<String> {
    let raw = &source[attr.span.clone()];
    let raw_value = raw.split_once('=').map(|(_, value)| value.trim_start())?;
    let quote = if raw_value.starts_with('\'') {
        '\''
    } else {
        '"'
    };
    let marker = |name: &str, value: &str| format!("{name}={quote}{value}{quote}");
    let ident = attr.value.as_deref().and_then(rust_ident);
    let id = |map: &BTreeMap<String, crate::artifact::Resource>, collection: bool| {
        ident
            .as_ref()
            .and_then(|ident| map.get(ident))
            .filter(|resource| (resource.role == SlotRole::Collection) == collection)
            .map(|resource| resource.id.to_string())
    };
    match attr.name.as_str() {
        "data-hemx-slot" => Some(marker("data-hemx-resource", &id(&resources.slots, false)?)),
        "data-hemx-collection" => Some(format!(
            "{} {}",
            marker("data-hemx-resource", &id(&resources.slots, true)?),
            marker("data-hemx-collection", "")
        )),
        "data-hemx-handle" => Some(marker("data-hemx-action", &id(&resources.handles, false)?)),
        "data-hemx-form" => {
            let form = resources.forms.get(ident.as_ref()?)?;
            Some(marker("data-hemx-resource", &form.resource.id.to_string()))
        }
        // `h-key` on an ordinary element is the authored spelling of a row key.
        "h-key" => Some(format!("data-hemx-key={raw_value}")),
        _ => None,
    }
}

/// A handled form carries its handle and build so native and enhanced
/// submissions are both dispatched and checked by the server.
fn action_fields(
    resources: &Resources,
    fingerprint: u64,
    surface: &SurfaceDocument,
    node: &SurfaceNode,
) -> Option<Edit> {
    let handle = static_attr(&node.attrs, "data-hemx-handle")?;
    let handle = resources.handles.get(&rust_ident(&handle)?)?;
    let offset = node.content_start?;
    let authored = |name: &str| {
        surface
            .forms
            .iter()
            .find(|form| form.node == node.id)
            .is_some_and(|form| form.controls.iter().any(|control| control.name == name))
    };
    let mut text = String::new();
    if !authored("__h") {
        text.push_str(&format!(
            "<input type=\"hidden\" name=\"__h\" value=\"{}\">",
            handle.id
        ));
    }
    if !authored("__b") {
        text.push_str(&format!(
            "<input type=\"hidden\" name=\"__b\" value=\"{fingerprint}\">"
        ));
    }
    (!text.is_empty()).then_some(Edit {
        range: offset..offset,
        text,
    })
}

use crate::artifact::{rust_ident, static_attr};
