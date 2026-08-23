use hemx_core::{
    event, navigate, redirect, replace, Atom, AtomSnapshot, AtomState, BuildFingerprint,
    ComponentRef, CssClass, CssClasses, Effect, EffectBatch, EventName, Form, FormError, FormValue,
    Handle, IntoEffect, KeyedSlot, NavigateMode, ParamName, Payload, ResourceId, ResourceKind,
    ResourceRef, SafeHtml, ScopeKey, ScrollBehavior, Slot, WireError,
};

#[test]
fn effect_batch_wire_round_trips() {
    let count = Slot::<u32>::new(1);
    let todos = KeyedSlot::<u64, String>::new(2);
    let user = Atom::<String>::new(3);

    let batch = (
        count.text(2),
        todos.append_text(7, String::from("Buy milk")),
        todos.replace_text(7, String::from("Buy oat milk")),
        user.set("Ada"),
        navigate("/todos"),
        event("toast", "Saved"),
    )
        .into_batch(BuildFingerprint(42));

    let bytes = batch.to_wire();
    assert_eq!(&bytes[..4], b"HEMX");
    assert_eq!(batch.encoded_len(), bytes.len()); //
    let decoded = EffectBatch::from_wire(&bytes).unwrap();

    assert_eq!(decoded, batch);
    assert!(decoded.is_compatible());
}

#[test]
fn compatibility_fixture_accepts_only_the_declared_v1_wire_version() {
    const V1_EMPTY_BATCH: &[u8] = &[
        72, 69, 77, 88, // HEMX
        1, 0, // ABI v1
        0, 0, // reserved
        7, 0, 0, 0, 0, 0, 0, 0, // fingerprint
        0, 0, 0, 0, // zero operations
    ];
    assert_eq!(
        EffectBatch::from_wire(V1_EMPTY_BATCH).unwrap().to_wire(),
        V1_EMPTY_BATCH
    );

    let mut future = V1_EMPTY_BATCH.to_vec();
    future[4] = 2;
    let future = EffectBatch::from_wire(&future).unwrap();
    assert_eq!(future.abi_version, 2);
    assert!(!future.is_compatible());
}

#[test]
fn canonical_wire_covers_every_closed_variant_and_rejects_truncation() {
    let slot = ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 0x0403_0201));
    let atom = ResourceRef::scoped(
        ResourceId::new(ResourceKind::Atom, 0x0807_0605),
        ScopeKey::KeyValue(String::from("row")),
    );
    let handle = ResourceRef::scoped(
        ResourceId::new(ResourceKind::Handle, 0x0c0b_0a09),
        ScopeKey::Field(String::from("email")),
    );
    let form = ResourceRef::unscoped(ResourceId::new(ResourceKind::Form, 0x100f_0e0d));
    let batch = EffectBatch {
        abi_version: hemx_core::EFFECT_BATCH_ABI_VERSION,
        fingerprint: BuildFingerprint(0x0807_0605_0403_0201),
        ops: vec![
            Effect::Put {
                target: slot.clone(),
                payload: Payload::Text(String::from("text")),
            },
            Effect::Put {
                target: handle.clone(),
                payload: Payload::Html(String::from("<p>safe</p>")),
            },
            Effect::Insert {
                target: atom.clone(),
                key: String::from("insert"),
                payload: Payload::Text(String::from("one")),
            },
            Effect::Prepend {
                target: form.clone(),
                key: String::from("prepend"),
                payload: Payload::Html(String::from("two")),
            },
            Effect::Remove {
                target: slot.clone(),
                key: None,
            },
            Effect::Remove {
                target: atom,
                key: Some(String::from("remove")),
            },
            Effect::Move {
                target: handle.clone(),
                key: String::from("move"),
                before: Some(String::from("before")),
            },
            Effect::Focus { target: form },
            Effect::Navigate {
                url: String::from("/push"),
                mode: NavigateMode::Push,
                scroll: ScrollBehavior::Preserve,
                title: None,
            },
            Effect::Navigate {
                url: String::from("/replace"),
                mode: NavigateMode::Replace,
                scroll: ScrollBehavior::Top,
                title: Some(String::from("Replace")),
            },
            Effect::Navigate {
                url: String::from("/redirect"),
                mode: NavigateMode::Redirect,
                scroll: ScrollBehavior::Element(handle),
                title: Some(String::from("Redirect")),
            },
            Effect::Emit {
                name: String::from("notice"),
                payload: String::from("saved"),
            },
        ],
    };

    let bytes = batch.to_wire();
    assert_eq!(&bytes[..4], b"HEMX");
    assert_eq!(batch.encoded_len(), bytes.len());
    assert_eq!(EffectBatch::from_wire(&bytes), Ok(batch));
    for end in 0..bytes.len() {
        assert_eq!(
            EffectBatch::from_wire(&bytes[..end]),
            Err(WireError::Truncated),
            "prefix ending at byte {end} must fail closed"
        );
    }
}

#[test]
fn canonical_wire_rejects_corrupt_tags_utf8_magic_and_trailing_bytes() {
    const BATCH_HEADER_LEN: usize = 4 + 4 + 8 + 4;
    const PUT_EFFECT_TAG: usize = BATCH_HEADER_LEN;
    const PUT_RESOURCE_KIND_TAG: usize = PUT_EFFECT_TAG + 1;
    const PUT_SCOPE_TAG: usize = PUT_RESOURCE_KIND_TAG + 1 + 4;
    const PUT_PAYLOAD_TAG: usize = PUT_SCOPE_TAG + 1;

    let put = EffectBatch {
        abi_version: 1,
        fingerprint: BuildFingerprint(1),
        ops: vec![Effect::Put {
            target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 1)),
            payload: Payload::Text(String::from("value")),
        }],
    }
    .to_wire();
    for offset in [
        PUT_EFFECT_TAG,
        PUT_RESOURCE_KIND_TAG,
        PUT_SCOPE_TAG,
        PUT_PAYLOAD_TAG,
    ] {
        let mut corrupt = put.clone();
        corrupt[offset] = 0xff;
        assert_eq!(EffectBatch::from_wire(&corrupt), Err(WireError::UnknownTag));
    }

    const NAVIGATE_MODE_TAG: usize = BATCH_HEADER_LEN + 1 + 4;
    const NAVIGATE_SCROLL_TAG: usize = NAVIGATE_MODE_TAG + 1;
    const NAVIGATE_TITLE_OPTION_TAG: usize = NAVIGATE_SCROLL_TAG + 1;
    let navigate = EffectBatch {
        abi_version: 1,
        fingerprint: BuildFingerprint(1),
        ops: vec![Effect::Navigate {
            url: String::new(),
            mode: NavigateMode::Push,
            scroll: ScrollBehavior::Preserve,
            title: None,
        }],
    }
    .to_wire();
    for offset in [
        NAVIGATE_MODE_TAG,
        NAVIGATE_SCROLL_TAG,
        NAVIGATE_TITLE_OPTION_TAG,
    ] {
        let mut corrupt = navigate.clone();
        corrupt[offset] = 0xff;
        assert_eq!(EffectBatch::from_wire(&corrupt), Err(WireError::UnknownTag));
    }

    let mut bad_magic = put.clone();
    bad_magic[0] = b'X';
    assert_eq!(EffectBatch::from_wire(&bad_magic), Err(WireError::BadMagic));

    const PUT_TEXT_START: usize = PUT_PAYLOAD_TAG + 1 + 4;
    let mut invalid_utf8 = put.clone();
    invalid_utf8[PUT_TEXT_START] = 0xff;
    assert_eq!(
        EffectBatch::from_wire(&invalid_utf8),
        Err(WireError::InvalidUtf8)
    );

    let mut trailing = put;
    trailing.push(0);
    assert_eq!(
        EffectBatch::from_wire(&trailing),
        Err(WireError::TrailingBytes)
    );
}

#[test]
fn optional_effects_compose_into_batches() {
    let count = Slot::<u32>::new(1);
    let batch = (Some(count.text(2)), Option::<Effect>::None).into_batch(BuildFingerprint(42));

    assert_eq!(batch.ops.len(), 1);
    assert_eq!(batch.ops[0], count.text(2));
}

#[test]
fn effect_collections_compose_into_batches() {
    let rows = KeyedSlot::<u64, String>::new(2);
    let summary = Slot::<u32>::new(3);
    let notice = Slot::<String>::new(4);

    let dynamic_rows = [7, 8]
        .into_iter()
        .map(|id| rows.replace_text(id, format!("todo {id}")))
        .collect::<Vec<_>>();
    let fixed_notices = [notice.text("Saved"), notice.text("Synced")];

    let batch =
        (dynamic_rows, Some(summary.text(2)), fixed_notices).into_batch(BuildFingerprint(42));

    assert_eq!(batch.ops.len(), 5);
    assert_eq!(batch.ops[0], rows.replace_text(7, String::from("todo 7")));
    assert_eq!(batch.ops[1], rows.replace_text(8, String::from("todo 8")));
    assert_eq!(batch.ops[2], summary.text(2));
    assert_eq!(batch.ops[3], notice.text("Saved"));
    assert_eq!(batch.ops[4], notice.text("Synced"));
}

#[test]
fn keyed_slot_replace_uses_scoped_resource_ref() {
    let todos = KeyedSlot::<u64, String>::new(9);
    let effect = todos.replace_text(12, String::from("done"));

    let Effect::Put { target, payload } = effect else {
        panic!("expected Put");
    };

    assert_eq!(target.resource.kind, ResourceKind::Slot);
    assert_eq!(target.resource.id, 9);
    assert_eq!(target.scope, Some(ScopeKey::KeyValue(String::from("12"))));
    assert_eq!(payload, Payload::Text(String::from("done")));
}

#[test]
fn tuple_composition_supports_arity_twelve() {
    let slot = Slot::<u8>::new(1);
    let batch = (
        slot.text(1),
        slot.text(2),
        slot.text(3),
        slot.text(4),
        slot.text(5),
        slot.text(6),
        slot.text(7),
        slot.text(8),
        slot.text(9),
        slot.text(10),
        slot.text(11),
        slot.text(12),
    )
        .into_batch(BuildFingerprint(1));

    assert_eq!(batch.ops.len(), 12);
}

#[test]
fn generated_form_helpers_target_form_fields() {
    let signup = Form::<()>::new(4);

    let Effect::Emit { name, payload } = signup.error("email", "Use your work email") else {
        panic!("expected Emit");
    };

    assert_eq!(name, "hemx:form-error");
    assert_eq!(payload, "4\u{1f}email\u{1f}Use your work email");

    let Effect::Focus { target } = signup.focus("email") else {
        panic!("expected Focus");
    };
    assert_eq!(target.scope, Some(ScopeKey::Field(String::from("email"))));
}

#[test]
fn generated_resource_helpers_preserve_target_keys_and_navigation_modes() {
    let rows = KeyedSlot::<u64, String>::new(9);
    let expected = ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 9));
    assert_eq!(
        rows.replace_html(12, SafeHtml::trusted("<li>done</li>")),
        Effect::Put {
            target: ResourceRef::scoped(expected.resource, ScopeKey::KeyValue("12".into())),
            payload: Payload::Html("<li>done</li>".into()),
        }
    );
    assert_eq!(
        rows.remove(12),
        Effect::Remove {
            target: expected.clone(),
            key: Some("12".into()),
        }
    );
    assert_eq!(
        rows.move_before(12, 13),
        Effect::Move {
            target: expected.clone(),
            key: "12".into(),
            before: Some("13".into()),
        }
    );
    assert_eq!(
        rows.move_to_end(12),
        Effect::Move {
            target: expected,
            key: "12".into(),
            before: None,
        }
    );

    let form = Form::<()>::new(4);
    assert_eq!(
        form.clear_field("email"),
        Effect::Put {
            target: ResourceRef::scoped(
                ResourceId::new(ResourceKind::Form, 4),
                ScopeKey::Field("email".into()),
            ),
            payload: Payload::Text(String::new()),
        }
    );
    assert_eq!(
        form.disable_while_pending(),
        Effect::Emit {
            name: "hemx:form-disable-while-pending".into(),
            payload: "4".into(),
        }
    );
    assert_eq!(form.clear(), form.reset());

    for (effect, expected_mode) in [
        (navigate("/push"), NavigateMode::Push),
        (hemx_core::push("/push"), NavigateMode::Push),
        (replace("/replace"), NavigateMode::Replace),
        (redirect("/redirect"), NavigateMode::Redirect),
    ] {
        let Effect::Navigate {
            mode,
            scroll,
            title,
            ..
        } = effect
        else {
            panic!("navigation helper must return Navigate");
        };
        assert_eq!(mode, expected_mode);
        assert_eq!(scroll, ScrollBehavior::Top);
        assert_eq!(title, None);
    }
}

#[test]
fn css_class_accumulation_preserves_existing_classes() {
    const A: CssClass = CssClass::new("a");
    const B: CssClass = CssClass::new("b");
    const C: CssClass = CssClass::new("c");
    assert_eq!(CssClasses::new([]).with(A).as_str(), "a");
    assert_eq!(CssClasses::from(A).with(B).with(C).as_str(), "a b c");
}

#[test]
fn slot_html_requires_explicit_safe_html() {
    let content = Slot::<String>::new(10);
    let Effect::Put { payload, .. } = content.html(SafeHtml::trusted("<strong>ok</strong>")) else {
        panic!("expected Put");
    };

    assert_eq!(payload, Payload::Html(String::from("<strong>ok</strong>")));
}

#[test]
fn safe_html_joins_only_explicit_safe_fragments() {
    let html = SafeHtml::join([
        SafeHtml::trusted("<main>"),
        SafeHtml::trusted("<strong>ok</strong>"),
        SafeHtml::trusted("</main>"),
    ]);

    assert_eq!(html.as_str(), "<main><strong>ok</strong></main>");
    assert_eq!(html.as_ref(), "<main><strong>ok</strong></main>");
    assert_eq!(html.to_string(), "<main><strong>ok</strong></main>");
}

#[test]
fn param_names_format_generated_param_names() {
    let param = ParamName::new("todo_id");
    assert_eq!(param.as_str(), "todo_id");
    assert_eq!(param.as_ref(), "todo_id");
    assert_eq!(param.to_string(), "todo_id");
}

#[test]
fn component_refs_format_generated_component_names() {
    let component = ComponentRef::new("todo_list");
    assert_eq!(component.as_str(), "todo_list");
    assert_eq!(component.as_ref(), "todo_list");
    assert_eq!(component.to_string(), "todo_list");
}

#[test]
fn generated_resources_format_for_hemplate_dynamic_attrs() {
    assert_eq!(Slot::<String>::new(10).to_string(), "10");
    assert_eq!(KeyedSlot::<u64, String>::new(11).to_string(), "11");
    assert_eq!(Atom::<String>::new(12).to_string(), "12");
    assert_eq!(Handle::<()>::new(42).to_string(), "42");
    assert_eq!(Form::<()>::new(13).to_string(), "13");
}

#[test]
fn generated_css_classes_join_for_hemplate_dynamic_class_attrs() {
    const CARD: CssClass = CssClass::new("work-card");
    const SELECTED: CssClass = CssClass::new("is-selected");

    let classes = CssClasses::from([CARD, SELECTED]);

    assert_eq!(classes.as_str(), "work-card is-selected");
    assert_eq!(classes.to_string(), "work-card is-selected");
    assert_eq!(CARD.with(SELECTED).as_str(), "work-card is-selected");
    assert_eq!(
        CARD.with_if(true, SELECTED).as_str(),
        "work-card is-selected"
    );
    assert_eq!(CARD.with_if(false, SELECTED).as_str(), "work-card");
}

#[test]
fn atom_state_bootstrap_is_postcard_round_trippable() {
    let state = AtomState {
        atoms: vec![AtomSnapshot {
            id: 7,
            bytes: vec![1, 2, 3],
        }],
    };

    let bytes = state.to_postcard().unwrap();
    assert_eq!(AtomState::from_postcard(&bytes).unwrap(), state);
}

#[test]
fn navigation_helpers_choose_explicit_modes() {
    let Effect::Navigate { mode, .. } = navigate("/docs") else {
        panic!("expected Navigate");
    };
    assert_eq!(mode, NavigateMode::Push);

    let Effect::Navigate { mode, .. } = replace("/docs") else {
        panic!("expected Navigate");
    };
    assert_eq!(mode, NavigateMode::Replace);

    let Effect::Navigate { mode, .. } = redirect("/login") else {
        panic!("expected Navigate");
    };
    assert_eq!(mode, NavigateMode::Redirect);
}

#[test]
fn build_fingerprint_is_deterministic_from_abi_parts() {
    let a = BuildFingerprint::from_parts(&[1, 2, 3, 4]);
    let b = BuildFingerprint::from_parts(&[1, 2, 3, 4]);
    let c = BuildFingerprint::from_parts(&[1, 2, 3, 5]);

    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_eq!(
        BuildFingerprint::from_parts(&[]),
        BuildFingerprint(0xcbf29ce484222325)
    );
    assert_eq!(a, BuildFingerprint(13725386680924731485));
    assert_eq!(hemx_core::EFFECT_BATCH_ABI_VERSION, 1);
    assert_eq!(hemx_core::SURFACE_SCHEMA_VERSION, 1);
    assert_eq!(hemx_core::RUNTIME_ABI_VERSION, 1);
}

#[test]
fn public_token_and_form_error_adapters_preserve_values() {
    const NOTICE: EventName = EventName::new("notice");
    assert_eq!(NOTICE.as_str(), "notice");
    assert_eq!(NOTICE.as_ref(), "notice");
    assert_eq!(NOTICE.to_string(), "notice");
    assert_eq!(String::from(NOTICE), "notice");
    assert_eq!(NOTICE.emit("saved"), event("notice", "saved"));

    const ACTIVE: CssClass = CssClass::new("active");
    assert_eq!(ACTIVE.as_str(), "active");
    assert_eq!(ACTIVE.as_ref(), "active");
    assert_eq!(ACTIVE.to_string(), "active");
    let classes = CssClasses::from(ACTIVE).with(CssClass::new("selected"));
    assert_eq!(classes.as_ref(), "active selected");

    let error = FormError::new("invalid email");
    assert_eq!(error.message(), "invalid email");
    assert_eq!(error.to_string(), "invalid email");
    assert_eq!(u32::parse_form_value("42"), Ok(42));
    assert_eq!(
        u32::parse_form_value("nope"),
        Err("invalid form value".into())
    );
    assert_eq!(
        Form::<()>::new(7).reset(),
        Effect::Emit {
            name: "hemx:form-reset".into(),
            payload: "7".into(),
        }
    );
}
