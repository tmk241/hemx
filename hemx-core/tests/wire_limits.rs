use hemx_core::{
    BuildFingerprint, Effect, EffectBatch, HistoryMode, IntoEffect, PatchMode, ResourceId,
    ResourceKind, ResourceRef, RowStep, SafeHtml, ScopeKey, ScrollBehavior, ScrollBlock, Slot,
    WireError, EFFECT_BATCH_ABI_VERSION, MAX_BATCH_BYTES, MAX_EFFECTS, MAX_FIELD_BYTES,
    MAX_ROW_KEYS,
};

fn delta(steps: Vec<RowStep>, rows: Vec<(String, SafeHtml)>) -> Effect {
    Effect::RowsDelta {
        target: target(),
        from: 1,
        to: 2,
        steps,
        rows,
    }
}

fn target() -> ResourceRef {
    ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 1))
}

fn batch(ops: Vec<Effect>) -> EffectBatch {
    ops.into_batch(BuildFingerprint(7))
}

// Each case isolates one string/byte location and records its length-prefix offset.
fn field_cases(value: &str) -> Vec<(String, Effect, usize)> {
    let mut cases = vec![
        (
            "patch HTML".into(),
            Effect::Patch {
                target: target(),
                mode: PatchMode::Morph,
                html: SafeHtml::trusted(value),
            },
            28,
        ),
        (
            "rows order key".into(),
            Effect::Rows {
                target: target(),
                order: vec![value.into()],
                rows: vec![],
            },
            31,
        ),
        (
            "rows HTML".into(),
            Effect::Rows {
                target: target(),
                order: vec!["a".into()],
                rows: vec![("a".into(), SafeHtml::trusted(value))],
            },
            45,
        ),
        // Opcode 20, target 21..27, from/to 27..43, step count 43..47, first step 47.
        (
            "delta removed key".into(),
            delta(vec![RowStep::Remove(value.into())], vec![]),
            48,
        ),
        (
            "delta placed key".into(),
            delta(
                vec![RowStep::Place {
                    key: value.into(),
                    before: None,
                }],
                vec![],
            ),
            48,
        ),
        (
            "delta anchor key".into(),
            delta(
                vec![RowStep::Place {
                    key: "a".into(),
                    before: Some(value.into()),
                }],
                vec![],
            ),
            54,
        ),
        (
            "delta row key".into(),
            delta(vec![], vec![(value.into(), SafeHtml::trusted(""))]),
            51,
        ),
        (
            "delta row HTML".into(),
            delta(vec![], vec![("a".into(), SafeHtml::trusted(value))]),
            56,
        ),
        (
            "visit URL".into(),
            Effect::Visit {
                url: value.into(),
                history: HistoryMode::Push,
            },
            21,
        ),
        (
            "dispatch event".into(),
            Effect::Dispatch {
                event: value.into(),
                payload: vec![],
            },
            21,
        ),
        (
            "dispatch payload".into(),
            Effect::Dispatch {
                event: String::new(),
                payload: value.as_bytes().to_vec(),
            },
            25,
        ),
    ];
    for scope in [
        ScopeKey::KeyValue(value.into()),
        ScopeKey::Field(value.into()),
    ] {
        let scoped = ResourceRef::scoped(target().resource, scope.clone());
        for effect in [
            Effect::Patch {
                target: scoped.clone(),
                mode: PatchMode::Replace,
                html: SafeHtml::trusted(""),
            },
            Effect::Rows {
                target: scoped.clone(),
                order: vec![],
                rows: vec![],
            },
            Effect::RowsDelta {
                target: scoped.clone(),
                from: 0,
                to: 0,
                steps: vec![],
                rows: vec![],
            },
            Effect::Focus {
                target: scoped.clone(),
                prevent_scroll: false,
            },
            Effect::Scroll {
                target: scoped.clone(),
                block: ScrollBlock::Start,
                behavior: ScrollBehavior::Auto,
            },
        ] {
            let name = format!(
                "target scope {:?} in {}",
                core::mem::discriminant(&scope),
                cases.len()
            );
            cases.push((name, effect, 27));
        }
    }
    cases
}

#[test]
fn every_nested_field_accepts_exact_budget_and_rejects_one_over() {
    assert_eq!(
        (MAX_BATCH_BYTES, MAX_FIELD_BYTES, MAX_EFFECTS),
        (1_048_576, 262_144, 1_024)
    );
    let exact = "x".repeat(MAX_FIELD_BYTES);
    let oversized = "x".repeat(MAX_FIELD_BYTES + 1);
    for ((name, effect, offset), (_, too_large, _)) in
        field_cases(&exact).into_iter().zip(field_cases(&oversized))
    {
        let original = batch(vec![effect]);
        let bytes = original
            .to_wire()
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(bytes.len(), original.encoded_len(), "{name}");
        assert_eq!(EffectBatch::from_wire(&bytes), Ok(original), "{name}");
        assert_eq!(
            batch(vec![too_large]).to_wire(),
            Err(WireError::FieldTooLarge),
            "{name}"
        );

        let mut one_over = bytes.clone();
        one_over[offset..offset + 4].copy_from_slice(&((MAX_FIELD_BYTES + 1) as u32).to_le_bytes());
        one_over.insert(offset + 4, b'x');
        assert_eq!(
            EffectBatch::from_wire(&one_over),
            Err(WireError::FieldTooLarge),
            "{name}"
        );

        // Reject a hostile declared length even without its purported payload.
        let mut hostile = bytes[..offset + 4].to_vec();
        hostile[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            EffectBatch::from_wire(&hostile),
            Err(WireError::FieldTooLarge),
            "{name}"
        );
    }
}

#[test]
fn field_budgets_count_utf8_bytes_not_characters() {
    let exact = "é".repeat(MAX_FIELD_BYTES / 2);
    let valid = batch(vec![Effect::Visit {
        url: exact.clone(),
        history: HistoryMode::Push,
    }]);
    assert_eq!(EffectBatch::from_wire(&valid.to_wire().unwrap()), Ok(valid));
    assert_eq!(
        batch(vec![Effect::Visit {
            url: exact + "x",
            history: HistoryMode::Push
        }])
        .to_wire(),
        Err(WireError::FieldTooLarge)
    );
}

#[test]
fn batch_byte_and_effect_budgets_accept_exact_and_reject_one_over() {
    // Four Dispatch effects have 20 envelope bytes and 9 framing bytes each.
    let mut original = batch(
        (0..4)
            .map(|i| Effect::Dispatch {
                event: String::new(),
                payload: vec![
                    0;
                    if i < 3 {
                        MAX_FIELD_BYTES
                    } else {
                        MAX_FIELD_BYTES - 56
                    }
                ],
            })
            .collect(),
    );
    let bytes = original.to_wire().unwrap();
    assert_eq!(bytes.len(), MAX_BATCH_BYTES);
    assert_eq!(original.encoded_len(), MAX_BATCH_BYTES);
    assert_eq!(EffectBatch::from_wire(&bytes), Ok(original.clone()));
    if let Effect::Dispatch { payload, .. } = &mut original.ops[3] {
        payload.push(0);
    }
    assert_eq!(original.encoded_len(), MAX_BATCH_BYTES + 1);
    assert_eq!(original.to_wire(), Err(WireError::BatchTooLarge));
    let mut bytes = bytes;
    bytes.push(0);
    assert_eq!(
        EffectBatch::from_wire(&bytes),
        Err(WireError::BatchTooLarge)
    );

    let focus = Effect::Focus {
        target: target(),
        prevent_scroll: false,
    };
    let mut original = batch(vec![focus.clone(); MAX_EFFECTS]);
    let mut bytes = original.to_wire().unwrap();
    assert_eq!(EffectBatch::from_wire(&bytes), Ok(original.clone()));
    original.ops.push(focus);
    assert_eq!(original.to_wire(), Err(WireError::TooManyEffects));
    bytes[16..20].copy_from_slice(&((MAX_EFFECTS + 1) as u32).to_le_bytes());
    assert_eq!(
        EffectBatch::from_wire(&bytes),
        Err(WireError::TooManyEffects)
    );
    bytes[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        EffectBatch::from_wire(&bytes[..20]),
        Err(WireError::TooManyEffects)
    );
}

#[test]
fn rows_abi_fixture_keeps_collection_instance_order_and_rows_separate() {
    let original = batch(vec![Effect::Rows {
        target: ResourceRef::scoped(target().resource, ScopeKey::KeyValue("old".into())),
        order: vec!["new".into(), "old".into()],
        rows: vec![("new".into(), SafeHtml::trusted("<li/>"))],
    }]);
    let mut expected = b"HEMX".to_vec();
    expected.extend_from_slice(&[3, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]);
    // opcode 1 (Rows), slot 1 scoped to collection instance "old"
    expected.extend_from_slice(&[1, 0, 1, 0, 0, 0, 1, 3, 0, 0, 0]);
    expected.extend_from_slice(b"old");
    expected.extend_from_slice(&[2, 0, 0, 0, 3, 0, 0, 0]);
    expected.extend_from_slice(b"new");
    expected.extend_from_slice(&[3, 0, 0, 0]);
    expected.extend_from_slice(b"old");
    expected.extend_from_slice(&[1, 0, 0, 0, 3, 0, 0, 0]);
    expected.extend_from_slice(b"new");
    expected.extend_from_slice(&[5, 0, 0, 0]);
    expected.extend_from_slice(b"<li/>");
    assert_eq!(original.to_wire().unwrap(), expected);
    assert_eq!(original.encoded_len(), expected.len());
    assert_eq!(EffectBatch::from_wire(&expected), Ok(original));
    assert_eq!(EFFECT_BATCH_ABI_VERSION, 3);
}

#[test]
fn malformed_nested_tags_and_lengths_are_rejected() {
    let original = batch(vec![Effect::Rows {
        target: target(),
        order: vec!["new".into()],
        rows: vec![("new".into(), SafeHtml::trusted("<li/>"))],
    }]);
    let bytes = original.to_wire().unwrap();
    // opcode, destination kind, destination scope.
    for offset in [20, 21, 26] {
        let mut corrupt = bytes.clone();
        corrupt[offset] = 255;
        assert_eq!(
            EffectBatch::from_wire(&corrupt),
            Err(WireError::UnknownTag),
            "offset {offset}"
        );
    }
    let mut truncated_field = bytes.clone();
    // The ordered key starts at byte 31 and declares more bytes than remain.
    truncated_field[31..35].copy_from_slice(&100_u32.to_le_bytes());
    assert_eq!(
        EffectBatch::from_wire(&truncated_field),
        Err(WireError::Truncated)
    );
    let mut invalid_utf8 = bytes.clone();
    invalid_utf8[35] = 255;
    assert_eq!(
        EffectBatch::from_wire(&invalid_utf8),
        Err(WireError::InvalidUtf8)
    );
    // More supplied rows than ordered keys cannot be a valid declaration.
    let mut overfull = bytes;
    overfull[38..42].copy_from_slice(&2_u32.to_le_bytes());
    assert_eq!(
        EffectBatch::from_wire(&overfull),
        Err(WireError::InvalidRows)
    );
}

#[test]
fn slot_rows_helper_declares_unscoped_collection_state() {
    let collection = Slot::<String>::new(1);
    assert_eq!(
        collection.rows_html([2, 1], [(1, SafeHtml::trusted("<li>1</li>"))]),
        Effect::Rows {
            target: target(),
            order: vec!["2".into(), "1".into()],
            rows: vec![("1".into(), SafeHtml::trusted("<li>1</li>"))],
        }
    );
}

#[test]
fn canonical_round_trips_and_sizes_cover_all_tag_values() {
    let mut ops = Vec::new();
    for kind in [ResourceKind::Slot, ResourceKind::Handle, ResourceKind::Form] {
        for scope in [
            None,
            Some(ScopeKey::KeyValue("row".into())),
            Some(ScopeKey::Field("field".into())),
        ] {
            let target = ResourceRef {
                resource: ResourceId::new(kind, 123),
                scope,
            };
            for mode in [PatchMode::Morph, PatchMode::Replace] {
                ops.push(Effect::Patch {
                    target: target.clone(),
                    mode,
                    html: SafeHtml::trusted("é"),
                });
            }
            for (order, rows) in [
                (vec![], vec![]),
                (vec![String::new(), "new-é".into()], vec![]),
                (
                    vec!["a".into(), "new-é".into()],
                    vec![("new-é".into(), SafeHtml::trusted("<li/>"))],
                ),
            ] {
                ops.push(Effect::Rows {
                    target: target.clone(),
                    order,
                    rows,
                });
            }
            for steps in [
                vec![],
                vec![
                    RowStep::Remove("gone-é".into()),
                    RowStep::Place {
                        key: "b".into(),
                        before: Some("a".into()),
                    },
                    RowStep::Place {
                        key: String::new(),
                        before: None,
                    },
                ],
            ] {
                ops.push(Effect::RowsDelta {
                    target: target.clone(),
                    from: u64::MAX,
                    to: 0x0123_4567_89ab_cdef,
                    steps,
                    rows: vec![("b".into(), SafeHtml::trusted("<li>é</li>"))],
                });
            }
            for prevent_scroll in [false, true] {
                ops.push(Effect::Focus {
                    target: target.clone(),
                    prevent_scroll,
                });
            }
            for block in [
                ScrollBlock::Start,
                ScrollBlock::Center,
                ScrollBlock::End,
                ScrollBlock::Nearest,
            ] {
                for behavior in [
                    ScrollBehavior::Auto,
                    ScrollBehavior::Instant,
                    ScrollBehavior::Smooth,
                ] {
                    ops.push(Effect::Scroll {
                        target: target.clone(),
                        block,
                        behavior,
                    });
                }
            }
        }
    }
    for history in [HistoryMode::Push, HistoryMode::Replace] {
        ops.push(Effect::Visit {
            url: "/é".into(),
            history,
        });
    }
    ops.push(Effect::Dispatch {
        event: "saved".into(),
        payload: vec![0, 255, 1],
    });
    let original = batch(ops);
    let bytes = original.to_wire().unwrap();
    assert_eq!(bytes.len(), original.encoded_len());
    assert_eq!(original.to_wire().unwrap(), bytes);
    let decoded = EffectBatch::from_wire(&bytes).unwrap();
    assert_eq!(decoded, original);
    assert_eq!(decoded.to_wire().unwrap(), bytes);
}

#[test]
fn row_deltas_bound_steps_and_reject_repeated_rows() {
    let too_many = delta(
        (0..=MAX_ROW_KEYS)
            .map(|key| RowStep::Remove(key.to_string()))
            .collect(),
        vec![],
    );
    assert_eq!(
        batch(vec![too_many]).to_wire(),
        Err(WireError::TooManyRowKeys)
    );
    let repeated = delta(
        vec![],
        vec![
            ("a".into(), SafeHtml::trusted("<li>1</li>")),
            ("a".into(), SafeHtml::trusted("<li>2</li>")),
        ],
    );
    assert_eq!(batch(vec![repeated]).to_wire(), Err(WireError::InvalidRows));

    // A declared step count over the bound is rejected before any step is read.
    let mut bytes = batch(vec![delta(vec![], vec![])]).to_wire().unwrap();
    let count_at = 43;
    bytes[count_at..count_at + 4].copy_from_slice(&(MAX_ROW_KEYS as u32 + 1).to_le_bytes());
    assert_eq!(
        EffectBatch::from_wire(&bytes),
        Err(WireError::TooManyRowKeys)
    );

    // Unknown step and anchor tags.
    let place = batch(vec![delta(
        vec![RowStep::Place {
            key: "a".into(),
            before: None,
        }],
        vec![],
    )]);
    for (at, tag) in [(47, 2u8), (53, 2u8)] {
        let mut bytes = place.to_wire().unwrap();
        bytes[at] = tag;
        assert_eq!(EffectBatch::from_wire(&bytes), Err(WireError::UnknownTag));
    }
}

#[test]
fn slot_delta_helper_names_both_orders_and_sends_only_the_steps() {
    let effect = Slot::<()>::new(9).rows_delta_html(
        ["a", "b", "c"],
        ["a", "c", "d"],
        [("d", SafeHtml::trusted("<li>d</li>"))],
    );
    let Effect::RowsDelta {
        from,
        to,
        steps,
        rows,
        ..
    } = effect
    else {
        panic!("a delta");
    };
    assert_eq!(from, hemx_core::order_hash(["a", "b", "c"]));
    assert_eq!(to, hemx_core::order_hash(["a", "c", "d"]));
    assert_eq!(
        steps,
        vec![
            RowStep::Remove("b".into()),
            RowStep::Place {
                key: "d".into(),
                before: None
            }
        ]
    );
    assert_eq!(rows.len(), 1);
}

#[test]
fn rows_and_deltas_count_their_framing_against_the_batch_budget() {
    // A batch at exactly the budget, then one more effect whose only bytes are
    // framing: encoding must refuse what decoding refuses.
    let mut full = batch(
        (0..4)
            .map(|i| Effect::Dispatch {
                event: String::new(),
                payload: vec![
                    0;
                    if i < 3 {
                        MAX_FIELD_BYTES
                    } else {
                        MAX_FIELD_BYTES - 56
                    }
                ],
            })
            .collect(),
    );
    assert_eq!(full.encoded_len(), MAX_BATCH_BYTES);
    assert!(full.to_wire().is_ok());
    let empty_rows = Effect::Rows {
        target: target(),
        order: vec![],
        rows: vec![],
    };
    let empty_delta = delta(vec![], vec![]);
    for effect in [empty_rows, empty_delta] {
        full.ops.push(effect);
        assert!(full.encoded_len() > MAX_BATCH_BYTES);
        let encoded = full.to_wire();
        assert!(matches!(encoded, Err(WireError::BatchTooLarge)));
        full.ops.pop();
    }
}
