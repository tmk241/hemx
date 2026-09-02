use hemx_core::{
    navigate, redirect, replace, BuildFingerprint, Effect, EffectBatch, EventName, Form,
    HistoryMode, InsertPosition, KeyedSlot, MovePosition, ParamName, PatchMode, ResourceId,
    ResourceKind, ResourceRef, SafeHtml, ScopeKey, ScrollBehavior, ScrollBlock, Slot, WireError,
    EFFECT_BATCH_ABI_VERSION,
};

fn reference(id: u32, scope: Option<ScopeKey>) -> ResourceRef {
    ResourceRef {
        resource: ResourceId::new(ResourceKind::Slot, id),
        scope,
    }
}

fn batch(ops: Vec<Effect>) -> EffectBatch {
    EffectBatch {
        abi_version: EFFECT_BATCH_ABI_VERSION,
        fingerprint: BuildFingerprint(7),
        ops,
    }
}

fn every_effect() -> Vec<Effect> {
    let target = reference(1, Some(ScopeKey::KeyValue(String::from("row"))));
    let sibling = reference(2, None);
    vec![
        Effect::Patch {
            target: target.clone(),
            mode: PatchMode::Morph,
            html: SafeHtml::trusted("<p>safe</p>"),
        },
        Effect::Patch {
            target: sibling.clone(),
            mode: PatchMode::Replace,
            html: SafeHtml::trusted("<p>replace</p>"),
        },
        Effect::Insert {
            target: target.clone(),
            position: InsertPosition::First,
            html: SafeHtml::trusted("<li>first</li>"),
        },
        Effect::Insert {
            target: target.clone(),
            position: InsertPosition::Last,
            html: SafeHtml::trusted("<li>last</li>"),
        },
        Effect::Insert {
            target: target.clone(),
            position: InsertPosition::Before(sibling.clone()),
            html: SafeHtml::trusted("<li>before</li>"),
        },
        Effect::Insert {
            target: sibling.clone(),
            position: InsertPosition::After(target.clone()),
            html: SafeHtml::trusted("<li>after</li>"),
        },
        Effect::Remove {
            target: target.clone(),
        },
        Effect::Move {
            target: target.clone(),
            position: MovePosition::Last,
        },
        Effect::Focus {
            target: target.clone(),
            prevent_scroll: true,
        },
        Effect::Scroll {
            target: target.clone(),
            block: ScrollBlock::Center,
            behavior: ScrollBehavior::Smooth,
        },
        Effect::Visit {
            url: String::from("/docs"),
            history: HistoryMode::Replace,
        },
        Effect::Dispatch {
            event: String::from("app:updated"),
            payload: vec![0, 1, 255],
        },
    ]
}

#[test]
fn effect_batch_wire_round_trips() {
    let original = batch(every_effect());
    let bytes = original.to_wire();
    assert_eq!(EffectBatch::from_wire(&bytes).unwrap(), original);
    assert_eq!(bytes.len(), original.encoded_len());
}

#[test]
fn canonical_wire_covers_every_closed_variant_and_rejects_truncation() {
    let original = batch(every_effect());
    assert_eq!(original.ops.len(), 12);

    let bytes = original.to_wire();
    for length in 0..bytes.len() {
        assert_eq!(
            EffectBatch::from_wire(&bytes[..length]),
            Err(WireError::Truncated)
        );
    }
}

#[test]
fn canonical_wire_rejects_corrupt_tags_utf8_magic_and_trailing_bytes() {
    let original = batch(vec![Effect::Visit {
        url: String::from("/same"),
        history: HistoryMode::Push,
    }]);
    let bytes = original.to_wire();
    assert_eq!(EffectBatch::from_wire(&bytes).unwrap().to_wire(), bytes);

    let mut bad_magic = bytes.clone();
    bad_magic[0] ^= 0xff;
    assert_eq!(EffectBatch::from_wire(&bad_magic), Err(WireError::BadMagic));

    let mut bad_tag = bytes.clone();
    bad_tag[20] = 0xff;
    assert_eq!(EffectBatch::from_wire(&bad_tag), Err(WireError::UnknownTag));

    let mut bad_utf8 = bytes.clone();
    let slash = bad_utf8.iter().position(|byte| *byte == b'/').unwrap();
    bad_utf8[slash] = 0xff;
    assert_eq!(
        EffectBatch::from_wire(&bad_utf8),
        Err(WireError::InvalidUtf8)
    );

    let mut trailing = bytes;
    trailing.push(0);
    assert_eq!(
        EffectBatch::from_wire(&trailing),
        Err(WireError::TrailingBytes)
    );
}

#[test]
fn compatibility_fixture_accepts_only_the_declared_v1_wire_version() {
    let compatible = batch(Vec::new());
    assert!(compatible.is_compatible());

    let mut incompatible = compatible.to_wire();
    incompatible[4..8].copy_from_slice(&(EFFECT_BATCH_ABI_VERSION + 1).to_le_bytes());
    assert_eq!(
        EffectBatch::from_wire(&incompatible),
        Err(WireError::UnsupportedVersion)
    );
}

#[test]
fn generated_resource_helpers_preserve_target_keys_and_navigation_modes() {
    let rows: KeyedSlot<String, String> = KeyedSlot::new(3);
    let append = rows.append_html("a".into(), SafeHtml::trusted("<li>a</li>"));
    assert!(matches!(
        append,
        Effect::Insert {
            target: ResourceRef {
                scope: Some(ScopeKey::KeyValue(key)),
                ..
            },
            position: InsertPosition::Last,
            ..
        } if key == "a"
    ));

    assert!(matches!(
        navigate("/push"),
        Effect::Visit {
            history: HistoryMode::Push,
            ..
        }
    ));
    assert!(matches!(
        replace("/replace"),
        Effect::Visit {
            history: HistoryMode::Replace,
            ..
        }
    ));
    assert!(matches!(
        redirect("/redirect"),
        Effect::Visit {
            history: HistoryMode::Replace,
            ..
        }
    ));
}

#[test]
fn slot_html_requires_explicit_safe_html() {
    let slot: Slot<String> = Slot::new(4);
    let effect = slot.html(SafeHtml::trusted("<strong>done</strong>"));
    assert!(matches!(
        effect,
        Effect::Patch {
            mode: PatchMode::Morph,
            html,
            ..
        } if html.as_str() == "<strong>done</strong>"
    ));
}

#[test]
fn generated_form_helpers_target_form_fields() {
    let form: Form<()> = Form::new(5);
    let effect = form.focus("email");
    assert!(matches!(
        effect,
        Effect::Focus {
            target: ResourceRef {
                resource: ResourceId { kind: ResourceKind::Form, id: 5 },
                scope: Some(ScopeKey::Field(field)),
            },
            prevent_scroll: false,
        } if field == "email"
    ));
}

#[test]
fn public_tokens_preserve_values() {
    assert_eq!(EventName::new("event").as_str(), "event");
    assert_eq!(ParamName::new("param").as_str(), "param");
}
