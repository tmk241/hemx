use hemx_core::{
    BuildFingerprint, Effect, EffectBatch, Form, GeneratedTarget, HistoryMode, KeyedSlot,
    PatchMode, ResourceId, ResourceKind, ResourceRef, SafeHtml, ScopeKey, Slot,
};

#[derive(Clone, Copy)]
struct TestTarget(ResourceKind, u32);

impl GeneratedTarget for TestTarget {
    fn __hemx_resource_id(self) -> ResourceId {
        ResourceId::new(self.0, self.1)
    }
}

fn panic_text<T>(result: std::thread::Result<T>) -> String {
    let panic = match result {
        Ok(_) => panic!("operation must panic"),
        Err(panic) => panic,
    };
    if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_owned()
    } else {
        panic!("panic payload was not text")
    }
}

#[test]
fn inspects_tuple_effects() {
    let count = Slot::<u32>::new(1);

    let inspected = hemx_test::run(|value| count.text(value), 42);

    assert!(inspected.has_slot(count));
    assert!(inspected.contains(&Effect::Patch {
        target: ResourceRef::unscoped(count.id()),
        mode: PatchMode::Morph,
        html: SafeHtml::trusted("42"),
    }));
}

#[test]
fn text_update_condition_is_bound_to_the_expected_target() {
    let expected_target = TestTarget(ResourceKind::Slot, 42);
    let inspected = hemx_test::inspect(vec![
        Effect::Patch {
            target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 42)),
            mode: PatchMode::Morph,
            html: SafeHtml::trusted("wrong payload"),
        },
        Effect::Patch {
            target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 7)),
            mode: PatchMode::Morph,
            html: SafeHtml::trusted("expected fragment"),
        },
    ]);

    assert!(inspected.updates_text(expected_target));
    assert!(inspected.payload_contains("expected fragment"));
    assert!(!inspected.updates_text_containing(expected_target, "expected fragment"));

    let panic = std::panic::catch_unwind(|| {
        inspected.assert_updates_text_containing(expected_target, "expected fragment");
    })
    .expect_err("a payload on another target must not satisfy the assertion");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .expect("assertion panic must carry a message");

    assert!(message.contains("expected fragment"), "{message}");
    assert!(message.contains("wrong payload"), "{message}");
    assert!(message.contains("ResourceId"), "{message}");
}

#[test]
fn html_update_assertion_reports_expectation_and_actual_effects() {
    let target = TestTarget(ResourceKind::Slot, 42);
    let inspected = hemx_test::inspect(Effect::Patch {
        target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 7)),
        mode: PatchMode::Morph,
        html: SafeHtml::trusted("<p>actual</p>"),
    });

    let panic = std::panic::catch_unwind(|| {
        inspected.assert_updates_html_containing(target, "expected fragment");
    })
    .expect_err("a mismatched target and payload must fail");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .expect("assertion panic must carry a message");

    assert!(message.contains("expected fragment"), "{message}");
    assert!(message.contains("ResourceId"), "{message}");
    assert!(message.contains("actual"), "{message}");
}

#[test]
fn finds_keyed_slot_targets() {
    let rows = KeyedSlot::<u32, String>::new(9);
    let inspected = hemx_test::inspect(rows.replace_html(7, SafeHtml::trusted("row")));

    assert!(inspected.has_keyed_slot(rows));
    assert_eq!(inspected.ops().len(), 1);
}

#[test]
fn inspector_predicates_bind_operation_target_scope_kind_and_payload() {
    let expected = TestTarget(ResourceKind::Slot, 42);
    let other = TestTarget(ResourceKind::Slot, 7);
    let resource = ResourceId::new(ResourceKind::Slot, 42);
    let keyed_ref = ResourceRef::scoped(resource, ScopeKey::KeyValue("row-1".into()));
    let form = Form::<()>::new(11);
    let expected_emit = Effect::Dispatch {
        event: "saved".into(),
        payload: b"card 42 saved".to_vec(),
    };
    let inspected = hemx_test::inspect(vec![
        Effect::Patch {
            target: ResourceRef::unscoped(other.__hemx_resource_id()),
            mode: PatchMode::Morph,
            html: SafeHtml::trusted("decoy needle"),
        },
        Effect::Patch {
            target: ResourceRef::unscoped(resource),
            mode: PatchMode::Morph,
            html: SafeHtml::trusted("expected text"),
        },
        Effect::Patch {
            target: keyed_ref.clone(),
            mode: PatchMode::Replace,
            html: SafeHtml::trusted("<li data-hemx-key=\"row-1\">replacement</li>"),
        },
        // row-old is left out of the declared order, i.e. removed.
        Effect::Rows {
            target: ResourceRef::unscoped(resource),
            order: vec!["row-0".into(), "row-1".into(), "row-2".into()],
            rows: vec![
                ("row-2".into(), SafeHtml::trusted("<li>inserted</li>")),
                ("row-0".into(), SafeHtml::trusted("<li>prepended</li>")),
            ],
        },
        Effect::Focus {
            target: ResourceRef::unscoped(form.id()),
            prevent_scroll: false,
        },
        Effect::Visit {
            url: "/cards/42".into(),
            history: HistoryMode::Push,
        },
        expected_emit.clone(),
    ]);

    assert!(!inspected.is_empty());
    assert_eq!(inspected.op_count(), 7);
    assert!(inspected.contains(&expected_emit));
    assert!(!inspected.contains(&Effect::Dispatch {
        event: "saved".into(),
        payload: b"wrong".to_vec(),
    }));
    assert!(inspected.has_resource(resource));
    assert!(inspected.has_resource(other.__hemx_resource_id()));
    assert!(inspected.has_target(expected));
    assert!(inspected.has_target(other));
    assert!(!inspected.has_target(TestTarget(ResourceKind::Slot, 99)));
    assert!(inspected.updates_text(expected));
    assert!(inspected.updates_text_containing(expected, "expected"));
    assert!(!inspected.updates_text_containing(expected, "decoy"));
    assert!(inspected.updates_text(other));
    assert!(!inspected.updates_text(TestTarget(ResourceKind::Slot, 99)));
    assert!(inspected.updates_html(expected));
    assert!(inspected.updates_html_containing(expected, "replacement"));
    assert!(!inspected.updates_html_containing(expected, "missing"));
    assert!(!inspected.updates_html(TestTarget(ResourceKind::Slot, 99)));
    assert!(inspected.replaces_keyed_html_containing(expected, "row-1", "replacement"));
    assert!(!inspected.replaces_keyed_html_containing(expected, "row-2", "replacement"));
    assert!(inspected.declares_row_containing(expected, "row-2", "inserted"));
    assert!(!inspected.declares_row_containing(expected, "row-0", "inserted"));
    assert!(inspected.removes_key(expected, "row-old"));
    assert!(!inspected.removes_key(expected, "row-2"));
    assert!(inspected.pushes_to("/cards/42"));
    assert!(!inspected.pushes_to("/cards/7"));
    assert!(inspected.payload_contains("prepended"));
    assert!(!inspected.payload_contains("absent"));
    assert!(inspected.payload_excludes("absent"));
    assert!(!inspected.payload_excludes("expected"));
    assert!(inspected.payload_excludes_key("absent"));
    assert!(!inspected.payload_excludes_key("row-1"));
    assert_eq!(
        inspected.target_html_containing(expected, "replacement"),
        Some("<li data-hemx-key=\"row-1\">replacement</li>")
    );
    assert_eq!(
        inspected.target_html_containing(expected, "inserted"),
        Some("<li>inserted</li>")
    );
    assert_eq!(
        inspected.target_html_containing(expected, "prepended"),
        Some("<li>prepended</li>")
    );
    assert_eq!(inspected.target_html_containing(expected, "missing"), None);
    assert!(inspected.emits("saved", "card 42 saved"));
    assert!(!inspected.emits("saved", "wrong"));
    assert!(inspected.emits_containing("saved", "42"));
    assert!(!inspected.emits_containing("other", "42"));
    assert!(inspected.has_ref(&keyed_ref));
    assert!(inspected.has_ref(&ResourceRef::unscoped(resource)));
    assert!(!inspected.has_ref(&ResourceRef::scoped(
        resource,
        ScopeKey::Field("row-1".into()),
    )));
    assert!(inspected.has_slot(Slot::<()>::new(42)));
    assert!(!inspected.has_slot(Slot::<()>::new(99)));
    assert!(inspected.has_keyed_slot(KeyedSlot::<String, ()>::new(42)));
    assert!(!inspected.has_keyed_slot(KeyedSlot::<String, ()>::new(99)));
    assert!(inspected.has_form(form));
    assert!(!inspected.has_form(Form::<()>::new(12)));

    let empty = hemx_test::inspect(Vec::<Effect>::new());
    assert!(empty.is_empty());
    assert_eq!(empty.op_count(), 0);
}

#[test]
fn inspect_wire_reports_the_decode_failure_and_accepts_canonical_batches() {
    let batch = EffectBatch {
        abi_version: hemx_core::EFFECT_BATCH_ABI_VERSION,
        fingerprint: BuildFingerprint(9),
        ops: vec![Effect::Dispatch {
            event: "saved".into(),
            payload: b"ok".to_vec(),
        }],
    };
    assert!(hemx_test::inspect_wire(&batch.to_wire().unwrap()).emits("saved", "ok"));

    let message = panic_text(std::panic::catch_unwind(|| hemx_test::inspect_wire(b"bad")));
    assert!(message.contains("invalid hemx effect wire response: Truncated"));
}

#[test]
fn builds_generated_handle_form_bodies() {
    let handle = hemx_core::Handle::<()>::new(7);

    let body = hemx_test::handle_form_body(handle, &[("title", "hello world"), ("tag", "a&b")]);

    assert_eq!(body, "__h=7&title=hello+world&tag=a%26b");
    assert_eq!(
        hemx_test::handle_form_body(handle, &[("AZaz09-_.~", "AZaz09-_.~ /é")],),
        "__h=7&AZaz09-_.~=AZaz09-_.~+%2F%C3%A9"
    );
    assert_eq!(hemx_test::unknown_handle_form_body(99), "__h=99");
}

#[test]
fn row_deltas_are_inspected_like_full_declarations() {
    let rows = TestTarget(ResourceKind::Slot, 4);
    let delta = Slot::<()>::new(4).rows_delta_html(
        ["a", "old"],
        ["a", "new"],
        [(
            "new",
            SafeHtml::trusted("<li data-hemx-key=\"new\">added</li>"),
        )],
    );
    let inspected = hemx_test::inspect(delta);
    assert!(inspected.updates_html(rows));
    assert!(inspected.declares_row_containing(rows, "new", "added"));
    assert!(inspected.removes_key(rows, "old"));
    assert!(!inspected.removes_key(rows, "a"));
    assert_eq!(
        inspected.target_html_containing(rows, "added"),
        Some("<li data-hemx-key=\"new\">added</li>")
    );
    assert!(inspected.payload_contains("added"));
}
