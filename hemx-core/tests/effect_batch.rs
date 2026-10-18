use hemx_core::{
    fold_batches, navigate, redirect, replace, BuildFingerprint, Effect, EffectBatch, EventName,
    Form, HistoryMode, ParamName, PatchMode, ResourceId, ResourceKind, ResourceRef, SafeHtml,
    ScopeKey, ScrollBehavior, ScrollBlock, Slot, WireError, EFFECT_BATCH_ABI_VERSION,
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
        Effect::Rows {
            target: sibling.clone(),
            order: vec!["b".into(), "a".into()],
            rows: vec![("a".into(), SafeHtml::trusted("<li>a</li>"))],
        },
        Effect::Rows {
            target: sibling.clone(),
            order: Vec::new(),
            rows: Vec::new(),
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
    let bytes = original.to_wire().unwrap();
    assert_eq!(EffectBatch::from_wire(&bytes).unwrap(), original);
    assert_eq!(bytes.len(), original.encoded_len());
}

#[test]
fn canonical_wire_covers_every_closed_variant_and_rejects_truncation() {
    let original = batch(every_effect());
    assert_eq!(original.ops.len(), 8);

    let bytes = original.to_wire().unwrap();
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
    let bytes = original.to_wire().unwrap();
    assert_eq!(
        EffectBatch::from_wire(&bytes).unwrap().to_wire().unwrap(),
        bytes
    );

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
fn compatibility_fixture_accepts_only_the_declared_v3_wire_version() {
    let compatible = batch(Vec::new());
    assert!(compatible.is_compatible());

    assert_eq!(EFFECT_BATCH_ABI_VERSION, 3);
    // ABI 2 (Insert/Remove/Move) is rejected explicitly, never reinterpreted.
    for version in [0, 1, 2, 4, u32::MAX] {
        let mut incompatible = compatible.clone();
        incompatible.abi_version = version;
        assert!(!incompatible.is_compatible());
        assert_eq!(incompatible.to_wire(), Err(WireError::UnsupportedVersion));
        let mut bytes = compatible.to_wire().unwrap();
        bytes[4..8].copy_from_slice(&version.to_le_bytes());
        assert_eq!(
            EffectBatch::from_wire(&bytes),
            Err(WireError::UnsupportedVersion)
        );
    }
}

#[test]
fn generated_resource_helpers_preserve_target_keys_and_navigation_modes() {
    let rows: Slot<String> = Slot::new(3);
    let declared = rows.rows_html(["b", "a"], [("a", SafeHtml::trusted("<li>a</li>"))]);
    assert_eq!(
        declared,
        Effect::Rows {
            target: ResourceRef::unscoped(rows.id()),
            order: vec!["b".into(), "a".into()],
            rows: vec![("a".into(), SafeHtml::trusted("<li>a</li>"))],
        }
    );

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
fn slot_text_escapes_markup_and_formats_once() {
    struct Text<'a>(&'a std::cell::Cell<usize>);
    impl core::fmt::Display for Text<'_> {
        fn fmt(&self, out: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            self.0.set(self.0.get() + 1);
            out.write_str("<b data-x=\"&\">é'</b>")
        }
    }
    let calls = std::cell::Cell::new(0);
    let Effect::Patch { html, .. } = Slot::<String>::new(4).text(Text(&calls)) else {
        panic!("text must produce a patch");
    };
    assert_eq!(
        html.as_str(),
        "&lt;b data-x=&quot;&amp;&quot;&gt;é&#39;&lt;/b&gt;"
    );
    assert_eq!(calls.get(), 1);
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

fn rows(order: &[&str], supplied: &[(&str, &str)]) -> Effect {
    Effect::Rows {
        target: reference(9, None),
        order: order.iter().map(|key| key.to_string()).collect(),
        rows: supplied
            .iter()
            .map(|(key, html)| (key.to_string(), SafeHtml::trusted(*html)))
            .collect(),
    }
}

fn patch(id: u32, html: &str) -> Effect {
    Effect::Patch {
        target: reference(id, None),
        mode: PatchMode::Morph,
        html: SafeHtml::trusted(html),
    }
}

#[test]
fn rows_reject_duplicate_and_unordered_keys_when_encoding_and_decoding() {
    for invalid in [
        rows(&["a", "a"], &[]),
        rows(&["a"], &[("b", "<li>b</li>")]),
        rows(&["a"], &[("a", "<li>a</li>"), ("a", "<li>a</li>")]),
    ] {
        assert_eq!(
            batch(vec![invalid.clone()]).to_wire(),
            Err(WireError::InvalidRows)
        );

        // Build the same bytes by hand from a valid twin, then corrupt the keys.
        let Effect::Rows {
            order,
            rows: supplied,
            ..
        } = &invalid
        else {
            unreachable!()
        };
        let mut bytes = batch(Vec::new()).to_wire().unwrap();
        bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 0]);
        bytes.extend_from_slice(&9u32.to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&(order.len() as u32).to_le_bytes());
        for key in order {
            bytes.extend_from_slice(&(key.len() as u32).to_le_bytes());
            bytes.extend_from_slice(key.as_bytes());
        }
        bytes.extend_from_slice(&(supplied.len() as u32).to_le_bytes());
        for (key, html) in supplied {
            for field in [key.as_bytes(), html.as_str().as_bytes()] {
                bytes.extend_from_slice(&(field.len() as u32).to_le_bytes());
                bytes.extend_from_slice(field);
            }
        }
        let decoded = EffectBatch::from_wire(&bytes);
        assert_eq!(decoded, Err(WireError::InvalidRows));
    }
    let valid = batch(vec![rows(&["b", "a"], &[("a", "<li>a</li>")])]);
    assert_eq!(
        EffectBatch::from_wire(&valid.to_wire().unwrap()).unwrap(),
        valid
    );
}

#[test]
fn rows_key_count_is_bounded_before_allocation() {
    let too_many: Vec<String> = (0..=hemx_core::MAX_ROW_KEYS)
        .map(|key| key.to_string())
        .collect();
    let declared = Effect::Rows {
        target: reference(9, None),
        order: too_many,
        rows: Vec::new(),
    };
    assert_eq!(
        batch(vec![declared]).to_wire(),
        Err(WireError::TooManyRowKeys)
    );

    let mut bytes = batch(Vec::new()).to_wire().unwrap();
    bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&[1, 0]);
    bytes.extend_from_slice(&9u32.to_le_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&(hemx_core::MAX_ROW_KEYS as u32 + 1).to_le_bytes());
    assert_eq!(
        EffectBatch::from_wire(&bytes),
        Err(WireError::TooManyRowKeys)
    );
    // An honest count with missing keys is truncation, not an allocation.
    let count_at = bytes.len() - 4;
    bytes[count_at..].copy_from_slice(&(hemx_core::MAX_ROW_KEYS as u32).to_le_bytes());
    assert_eq!(EffectBatch::from_wire(&bytes), Err(WireError::Truncated));
}

fn concatenated(run: &[EffectBatch]) -> EffectBatch {
    batch(run.iter().flat_map(|batch| batch.ops.clone()).collect())
}

fn assert_coalesces(run: &[EffectBatch]) {
    let expected = concatenated(run);
    assert_eq!(hemx_core::fold_run(run), Some(expected.clone()));
    assert_eq!(fold_batches(run.to_vec()), vec![expected.clone()]);
    assert_eq!(
        EffectBatch::from_wire(&expected.to_wire().unwrap()).unwrap(),
        expected
    );
}

#[test]
fn coalescing_preserves_every_patch_mode_and_intermediate_form_default() {
    for first in [PatchMode::Morph, PatchMode::Replace] {
        for second in [PatchMode::Morph, PatchMode::Replace] {
            let with = |mode, html| {
                batch(vec![Effect::Patch {
                    target: reference(1, None),
                    mode,
                    html: SafeHtml::trusted(html),
                }])
            };
            // A changed server default can reset a dirty input. Returning to
            // the original default must not erase that intermediate morph.
            assert_coalesces(&[
                with(first, "<input value=old>"),
                with(second, "<input value=new>"),
                with(PatchMode::Morph, "<input value=old>"),
            ]);
        }
    }
}

#[test]
fn coalescing_keeps_repeated_targets_and_distinct_or_nested_resources_in_order() {
    assert_coalesces(&[
        batch(vec![
            patch(1, "<section>parent</section>"),
            patch(2, "child"),
        ]),
        batch(vec![
            patch(1, "<input value=new>"),
            patch(1, "<input value=old>"),
        ]),
        batch(vec![patch(3, "a different resource")]),
    ]);
}

#[test]
fn coalescing_preserves_row_html_orders_removals_and_reintroductions() {
    assert_coalesces(&[
        batch(vec![rows(
            &["a", "b"],
            &[("a", "<li><input value=old></li>"), ("b", "<li>b</li>")],
        )]),
        batch(vec![
            rows(&["a", "b"], &[("a", "<li><input value=new></li>")]),
            rows(&["a"], &[]),
        ]),
        batch(vec![rows(
            &["b", "a"],
            &[("a", "<li><input value=old></li>"), ("b", "<li>new b</li>")],
        )]),
        batch(vec![rows(&["a", "b"], &[])]),
    ]);
}

#[test]
fn coalescing_preserves_each_delta_even_for_repeated_keys_and_hash_gaps() {
    let slot = Slot::<()>::new(9);
    let delta = |before: &[&str], after: &[&str], html| {
        batch(vec![slot.rows_delta_html(
            before.iter().copied(),
            after.iter().copied(),
            [("a", SafeHtml::trusted(html))],
        )])
    };
    let chain = [
        delta(&["a", "b"], &["b", "a"], "<li><input value=old></li>"),
        delta(&["b", "a"], &["a", "b"], "<li><input value=new></li>"),
        delta(&["a", "b"], &["b", "a"], "<li><input value=old></li>"),
    ];
    assert_coalesces(&chain);
    // Do not repair or conceal a gap: the client still checks each delta's
    // original precondition. Full declarations are also kept as separate ops.
    assert_coalesces(&[
        batch(vec![rows(&["a", "b"], &[("a", "<li>a</li>")])]),
        chain[0].clone(),
        delta(&["x", "a"], &["a", "x"], "<li>gap</li>"),
        batch(vec![rows(&["a"], &[])]),
        chain[1].clone(),
    ]);
}

#[test]
fn every_side_effect_is_a_barrier_on_either_side() {
    let declarations = batch(vec![patch(1, "x")]);
    for side_effect in every_effect().into_iter().filter(|effect| {
        matches!(
            effect,
            Effect::Focus { .. }
                | Effect::Scroll { .. }
                | Effect::Visit { .. }
                | Effect::Dispatch { .. }
        )
    }) {
        let barrier = batch(vec![patch(1, "y"), side_effect]);
        for pair in [
            [declarations.clone(), barrier.clone()],
            [barrier.clone(), declarations.clone()],
        ] {
            assert!(!hemx_core::folds_into(&pair[0], &pair[1]));
            assert_eq!(hemx_core::fold_run(&pair), None);
            assert_eq!(fold_batches(pair.clone()), pair.to_vec());
        }
        assert_eq!(hemx_core::fold_run(&[barrier]), None);
    }
}

#[test]
fn differing_builds_and_abis_are_barriers() {
    let original = batch(vec![patch(1, "x")]);
    let mut other_build = original.clone();
    other_build.fingerprint = BuildFingerprint(8);
    let mut other_abi = original.clone();
    other_abi.abi_version += 1;
    for other in [other_build, other_abi] {
        for pair in [
            [original.clone(), other.clone()],
            [other.clone(), original.clone()],
        ] {
            assert!(!hemx_core::folds_into(&pair[0], &pair[1]));
            assert_eq!(hemx_core::fold_run(&pair), None);
            assert_eq!(fold_batches(pair.clone()), pair.to_vec());
        }
    }
}

#[test]
fn empty_singleton_and_shared_runs_keep_their_operations() {
    assert_eq!(hemx_core::fold_run::<EffectBatch>(&[]), None);
    assert!(fold_batches([]).is_empty());
    assert_coalesces(&[batch(vec![])]);
    assert_coalesces(&[batch(vec![patch(1, "only")])]);
    assert_coalesces(&[batch(vec![]), batch(vec![patch(1, "only")]), batch(vec![])]);
    let shared: Vec<std::sync::Arc<EffectBatch>> = [
        batch(vec![rows(&["a"], &[("a", "<li>a</li>")])]),
        batch(vec![rows(&["a", "b"], &[("b", "<li>b</li>")])]),
    ]
    .into_iter()
    .map(std::sync::Arc::new)
    .collect();
    assert_eq!(
        hemx_core::fold_run(&shared),
        Some(batch(
            shared.iter().flat_map(|batch| batch.ops.clone()).collect()
        ))
    );
}

#[test]
fn coalescing_enforces_the_total_effect_limit_without_partial_folding() {
    let run = [
        batch(vec![patch(1, "x"); hemx_core::MAX_EFFECTS / 2]),
        batch(vec![patch(1, "y"); hemx_core::MAX_EFFECTS / 2]),
    ];
    assert_coalesces(&run);
    let oversized = [run[0].clone(), run[1].clone(), batch(vec![patch(1, "z")])];
    assert!(oversized.iter().all(|batch| batch.to_wire().is_ok()));
    assert!(hemx_core::folds_into(&oversized[1], &oversized[2]));
    assert_eq!(hemx_core::fold_run(&oversized), None);
    assert_eq!(fold_batches(oversized.clone()), oversized.to_vec());
}

#[test]
fn coalescing_enforces_the_total_byte_limit_including_one_envelope() {
    let overhead = batch(vec![patch(1, ""); 4]).encoded_len();
    let field = hemx_core::MAX_FIELD_BYTES;
    let remaining = hemx_core::MAX_BATCH_BYTES - overhead - 3 * field;
    let run = [
        batch(vec![patch(1, &"x".repeat(field)); 2]),
        batch(vec![
            patch(1, &"y".repeat(field)),
            patch(1, &"z".repeat(remaining)),
        ]),
    ];
    assert_eq!(concatenated(&run).encoded_len(), hemx_core::MAX_BATCH_BYTES);
    assert_coalesces(&run);
    let oversized = [
        run[0].clone(),
        batch(vec![
            patch(1, &"y".repeat(field)),
            patch(1, &"z".repeat(remaining + 1)),
        ]),
    ];
    assert!(oversized.iter().all(|batch| batch.to_wire().is_ok()));
    assert_eq!(hemx_core::fold_run(&oversized), None);
    assert_eq!(fold_batches(oversized.clone()), oversized.to_vec());
}

#[test]
fn coalescing_does_not_combine_per_declaration_row_step_limits() {
    // Each delta remains within its own row-step limit. Combining their steps
    // would exceed it, but two intact declarations fit the batch's limits.
    let delta = batch(vec![Effect::RowsDelta {
        target: reference(9, None),
        from: 1,
        to: 2,
        steps: vec![hemx_core::RowStep::Remove("a".into()); hemx_core::MAX_ROW_KEYS / 2 + 1],
        rows: vec![],
    }]);
    assert_coalesces(&[delta.clone(), delta]);
}
