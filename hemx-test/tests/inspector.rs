use hemx_core::{
    Atom, BuildFingerprint, Effect, EffectBatch, Form, GeneratedTarget, KeyedSlot, NavigateMode,
    Payload, ResourceId, ResourceKind, ResourceRef, ScopeKey, Slot,
};

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
    let user = Atom::<String>::new(2);

    let inspected = hemx_test::run(|value| (count.text(value), user.set("alice")), 42);

    assert!(inspected.has_slot(count));
    assert!(inspected.has_atom(user));
    assert!(inspected.contains(&Effect::Put {
        target: ResourceRef::unscoped(count.id()),
        payload: Payload::text(42),
    }));
}

#[test]
fn text_update_condition_is_bound_to_the_expected_target() {
    let expected_target = TestTarget(ResourceKind::Slot, 42);
    let inspected = hemx_test::inspect(vec![
        Effect::Put {
            target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 42)),
            payload: Payload::Text(String::from("wrong payload")),
        },
        Effect::Put {
            target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 7)),
            payload: Payload::Text(String::from("expected fragment")),
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
    let inspected = hemx_test::inspect(Effect::Put {
        target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 7)),
        payload: Payload::Html(String::from("<p>actual</p>")),
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
    let inspected = hemx_test::inspect(rows.append_text(7, String::from("row")));

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
    let expected_emit = Effect::Emit {
        name: "saved".into(),
        payload: "card 42 saved".into(),
    };
    let inspected = hemx_test::inspect(vec![
        Effect::Put {
            target: ResourceRef::unscoped(other.__hemx_resource_id()),
            payload: Payload::Text("decoy needle".into()),
        },
        Effect::Put {
            target: ResourceRef::unscoped(resource),
            payload: Payload::Text("expected text".into()),
        },
        Effect::Put {
            target: keyed_ref.clone(),
            payload: Payload::Html("<li data-key=\"row-1\">replacement</li>".into()),
        },
        Effect::Insert {
            target: ResourceRef::unscoped(resource),
            key: "row-2".into(),
            payload: Payload::Html("<li>inserted</li>".into()),
        },
        Effect::Prepend {
            target: ResourceRef::unscoped(resource),
            key: "row-0".into(),
            payload: Payload::Html("<li>prepended</li>".into()),
        },
        Effect::Remove {
            target: ResourceRef::unscoped(resource),
            key: Some("row-old".into()),
        },
        Effect::Focus {
            target: ResourceRef::unscoped(form.id()),
        },
        Effect::Navigate {
            url: "/cards/42".into(),
            mode: NavigateMode::Push,
            scroll: hemx_core::ScrollBehavior::Preserve,
            title: None,
        },
        expected_emit.clone(),
        Effect::Emit {
            name: "hemx:form-reset".into(),
            payload: form.id().id.to_string(),
        },
    ]);

    assert!(!inspected.is_empty());
    assert_eq!(inspected.op_count(), 10);
    assert!(inspected.contains(&expected_emit));
    assert!(!inspected.contains(&Effect::Emit {
        name: "saved".into(),
        payload: "wrong".into(),
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
    assert!(inspected.inserts_html_containing(expected, "row-2", "inserted"));
    assert!(!inspected.inserts_html_containing(expected, "row-0", "inserted"));
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
        Some("<li data-key=\"row-1\">replacement</li>")
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
    assert!(!inspected.has_atom(Atom::<()>::new(42)));
    assert!(inspected.has_form(form));
    assert!(!inspected.has_form(Form::<()>::new(12)));
    assert!(inspected.resets_form(form));
    assert!(!inspected.resets_form(Form::<()>::new(12)));

    let empty = hemx_test::inspect(Vec::<Effect>::new());
    assert!(empty.is_empty());
    assert_eq!(empty.op_count(), 0);
}

#[test]
fn selector_helpers_validate_parts_and_cover_unscoped_variants() {
    assert_eq!(hemx_test::heading_selector("", 2), "h2");
    assert_eq!(hemx_test::list_item_selector(""), "li");
    assert_eq!(hemx_test::prose_selector(""), "p");
    assert_eq!(
        hemx_test::target_selector(TestTarget(ResourceKind::Slot, 1)),
        "[data-sid=\"1\"]"
    );
    assert_eq!(
        hemx_test::target_selector(TestTarget(ResourceKind::Atom, 2)),
        "[data-aid=\"2\"]"
    );
    assert_eq!(
        hemx_test::target_selector(TestTarget(ResourceKind::Handle, 3)),
        "[data-hid=\"3\"]"
    );
    assert_eq!(
        hemx_test::target_selector(TestTarget(ResourceKind::Form, 4)),
        "[data-fid=\"4\"]"
    );

    assert_eq!(
        hemx_test::nav_link_selector("/path\\\"quoted"),
        "a[href=\"/path\\\\\\\"quoted\"]"
    );

    hemx_test::assert_rendered_target(
        TestTarget(ResourceKind::Slot, 1),
        "slot",
        "<p data-sid=\"1\"></p>",
    );
    hemx_test::assert_rendered_handle(
        hemx_core::Handle::<()>::new(3),
        "handle",
        "<button data-hid=\"3\"></button>",
    );
    assert!(std::panic::catch_unwind(|| {
        hemx_test::assert_rendered_target(TestTarget(ResourceKind::Slot, 1), "slot", "<p></p>");
    })
    .is_err());
    assert!(std::panic::catch_unwind(|| {
        hemx_test::assert_rendered_handle(
            hemx_core::Handle::<()>::new(3),
            "handle",
            "<button data-other=\"3\"></button>",
        );
    })
    .is_err());

    for invalid in ["", "two parts", ".class", "#id", "a>b", "a[b]"] {
        assert!(panic_text(std::panic::catch_unwind(|| {
            hemx_test::class_selector(invalid)
        }))
        .contains("class selector part"));
    }
    for (call, label) in [
        (
            std::panic::catch_unwind(|| hemx_test::element_class_selector("bad tag", "ok")),
            "element selector part",
        ),
        (
            std::panic::catch_unwind(|| hemx_test::element_class_selector("span", "bad class")),
            "class selector part",
        ),
        (
            std::panic::catch_unwind(|| hemx_test::class_child_selector("bad parent", "li", "row")),
            "parent class selector part",
        ),
        (
            std::panic::catch_unwind(|| hemx_test::class_child_selector("list", "bad tag", "row")),
            "element selector part",
        ),
        (
            std::panic::catch_unwind(|| hemx_test::class_child_selector("list", "li", "bad class")),
            "class selector part",
        ),
        (
            std::panic::catch_unwind(|| hemx_test::class_descendant_selector("bad parent", "i")),
            "parent class selector part",
        ),
        (
            std::panic::catch_unwind(|| hemx_test::class_descendant_selector("note", "bad tag")),
            "element selector part",
        ),
    ] {
        assert!(panic_text(call).contains(label));
    }
}

#[test]
fn inspect_wire_reports_the_decode_failure_and_accepts_canonical_batches() {
    let batch = EffectBatch {
        abi_version: hemx_core::EFFECT_BATCH_ABI_VERSION,
        fingerprint: BuildFingerprint(9),
        ops: vec![Effect::Emit {
            name: "saved".into(),
            payload: "ok".into(),
        }],
    };
    assert!(hemx_test::inspect_wire(&batch.to_wire()).emits("saved", "ok"));

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
fn builds_authoring_boundary_selectors() {
    assert_eq!(
        hemx_test::root_selector("techdemo"),
        r#"[data-hemx-root="techdemo"]"#
    );
    assert_eq!(
        hemx_test::island_selector("orbit"),
        r#"[data-hemx-island="orbit"]"#
    );
    assert_eq!(hemx_test::island_attribute_name(), "data-hemx-island");
    assert_eq!(hemx_test::island_event_name("orbit"), "hemx:island-orbit");
    assert_eq!(
        hemx_test::sse_endpoint_marker("/events"),
        r#"data-hemx-sse="/events""#
    );
    assert_eq!(hemx_test::any_root_selector(), "[data-hemx-root]");
    assert_eq!(
        hemx_test::root_element_selector("main", "docs"),
        r#"main[data-hemx-root="docs"]"#
    );
    assert_eq!(hemx_test::document_body_selector(), "body");
    assert_eq!(hemx_test::document_title_selector(), "title");
    assert_eq!(
        hemx_test::runtime_script_selector(),
        r#"script[src^="/hemx."][src$=".js"]"#
    );
    assert_eq!(
        hemx_test::target_selector(TestTarget(ResourceKind::Slot, 42)),
        r#"[data-sid="42"]"#
    );
    hemx_test::assert_rendered_target(
        TestTarget(ResourceKind::Slot, 42),
        "gallery::search_result",
        r#"<li data-sid="42">row</li>"#,
    );
    assert_eq!(
        hemx_test::handle_button_selector(hemx_core::Handle::<()>::new(7)),
        r#"button[data-hid="7"]"#
    );
    hemx_test::assert_rendered_handle(
        hemx_core::Handle::<()>::new(7),
        "gallery::search",
        r#"<button data-hid="7">Search</button>"#,
    );
    assert_eq!(hemx_test::article_selector(), "article");
    assert_eq!(hemx_test::strong_text_selector(), "strong");
    assert_eq!(hemx_test::small_text_selector(), "small");
    assert_eq!(hemx_test::escaped_markup_selector("b"), "b");
    assert_eq!(hemx_test::heading_selector("article", 1), "article h1");
    assert_eq!(hemx_test::list_item_selector("ul"), "ul li");
    assert_eq!(hemx_test::prose_selector("article"), "article p");
    assert_eq!(hemx_test::form_selector("header"), "header form");
    assert_eq!(
        hemx_test::select_options_selector("column"),
        r#"select[name="column"] > option"#
    );
    assert_eq!(hemx_test::class_selector("lane"), ".lane");
    assert_eq!(
        hemx_test::element_class_selector("span", "presence"),
        "span.presence"
    );
    assert_eq!(
        hemx_test::class_child_selector("columns", "section", "column"),
        ".columns > section.column"
    );
    assert_eq!(
        hemx_test::class_descendant_selector("impact", "i"),
        ".impact i"
    );
    assert_eq!(hemx_test::disabled_button_selector(), "button[disabled]");
    assert_eq!(
        hemx_test::nav_link_selector("/architecture"),
        r#"a[href="/architecture"]"#
    );
    assert_eq!(
        hemx_test::page_nav_link_selector("/docs"),
        r#"a[href="/docs"][data-hemx-nav]:not([data-hemx-handle])"#
    );
    assert_eq!(hemx_test::island_snapshot_marker(), "data-island-snapshot=");
    assert_eq!(
        hemx_test::island_readout_selector(),
        "[data-island-readout]"
    );
    assert_eq!(
        hemx_test::scoped_island_readout_selector("#probe-island"),
        "#probe-island [data-island-readout]"
    );
    assert_eq!(
        hemx_test::keyed_selector(".work-card", 4),
        r#".work-card[data-key="4"]"#
    );
    assert_eq!(hemx_test::keyed_items_selector("li"), "li[data-key]");

    let probe = hemx_test::island_probe_script(
        "probe-\"island",
        "orbit\\bridge",
        "line1\nline2\rline3\tend",
        "detail \"quoted\"",
    );
    assert!(probe.contains(r#"island.id = "probe-\"island";"#));
    assert!(probe.contains(r#"const islandName = "orbit\\bridge";"#));
    assert!(probe.contains(r#""line1\nline2\rline3\tend""#));
    assert!(probe.contains(r#"detail: "detail \"quoted\"""#));
}

#[derive(Clone, Copy)]
struct TestTarget(ResourceKind, u32);

impl GeneratedTarget for TestTarget {
    fn __hemx_resource_id(self) -> ResourceId {
        ResourceId::new(self.0, self.1)
    }
}
