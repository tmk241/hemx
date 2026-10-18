//! Canonical producer bytes for the independent browser boundary probe.
use hemx::advanced::{
    BuildFingerprint, Effect, EffectBatch, PatchMode, ResourceId, ResourceKind, ResourceRef,
    SafeHtml, WireError, EFFECT_BATCH_ABI_VERSION, MAX_BATCH_BYTES, MAX_EFFECTS, MAX_FIELD_BYTES,
};
use serde_json::{json, Value};

fn batch(ops: Vec<Effect>) -> EffectBatch {
    EffectBatch {
        abi_version: EFFECT_BATCH_ABI_VERSION,
        fingerprint: BuildFingerprint(0),
        ops,
    }
}

fn patch(text: &str) -> Effect {
    Effect::Patch {
        target: ResourceRef::unscoped(ResourceId {
            kind: ResourceKind::Slot,
            id: 1,
        }),
        mode: PatchMode::Morph,
        // Fixture literals contain only ASCII text, never application input.
        html: SafeHtml::trusted(text),
    }
}

fn dispatch(length: usize) -> Effect {
    Effect::Dispatch {
        event: "w".to_owned(),
        payload: vec![b'x'; length],
    }
}

fn encode(ops: Vec<Effect>) -> Vec<u8> {
    let value = batch(ops);
    let bytes = value.to_wire().expect("canonical browser boundary fixture");
    assert_eq!(EffectBatch::from_wire(&bytes).unwrap(), value);
    bytes
}

// Repeated payload bytes are transported as runs, not megabytes of JSON numbers.
// Compression happens only after the real encoder and native decoder agree.
fn packed(bytes: &[u8]) -> Value {
    let mut runs: Vec<(u8, usize)> = Vec::new();
    for &byte in bytes {
        match runs.last_mut() {
            Some((previous, count)) if *previous == byte => *count += 1,
            _ => runs.push((byte, 1)),
        }
    }
    json!({"length": bytes.len(), "runs": runs})
}

fn set_u32(bytes: &mut [u8], offset: usize, value: usize) {
    bytes[offset..offset + 4].copy_from_slice(&u32::try_from(value).unwrap().to_le_bytes());
}

fn negative(name: &str, bytes: Vec<u8>, expected: WireError) -> Value {
    assert_eq!(EffectBatch::from_wire(&bytes), Err(expected), "{name}");
    json!({"name": name, "wire": packed(&bytes)})
}

pub fn cases() -> Value {
    // ABI v2 envelope: magic[4], version[4], fingerprint[8], effect count[4].
    // Negative mutations below deliberately name their ABI offsets. All positive
    // bytes (including their field prefixes) come exclusively from to_wire().
    const HEADER: usize = 20;
    let small = encode(vec![patch("after")]);
    let two = encode(vec![patch("after"), patch("later")]);
    let second = small.len();
    let mut rejected = Vec::new();
    for (name, offset, value, error) in [
        ("magic", 0, b'X', WireError::BadMagic),
        ("abi", 4, 255, WireError::UnsupportedVersion),
        ("effect-tag", second, 255, WireError::UnknownTag),
        ("resource-tag", second + 1, 255, WireError::UnknownTag),
        ("scope-tag", second + 6, 255, WireError::UnknownTag),
        ("patch-tag", second + 7, 255, WireError::UnknownTag),
        ("utf8", second + 12, 255, WireError::InvalidUtf8),
    ] {
        let mut bytes = two.clone();
        bytes[offset] = value;
        rejected.push(negative(name, bytes, error));
    }
    rejected.push(negative(
        "truncated-header",
        two[..HEADER - 1].to_vec(),
        WireError::Truncated,
    ));
    rejected.push(negative(
        "truncated-payload",
        two[..two.len() - 1].to_vec(),
        WireError::Truncated,
    ));
    let mut trailing = two.clone();
    trailing.push(0);
    rejected.push(negative("trailing", trailing, WireError::TrailingBytes));
    let mut declared = two.clone();
    set_u32(&mut declared, second + 8, MAX_FIELD_BYTES);
    rejected.push(negative("declared-length", declared, WireError::Truncated));
    // Fingerprint mismatch is a browser/root compatibility rejection, not an
    // intrinsic Rust decoding error: it remains a valid canonical batch.
    let fingerprint = EffectBatch {
        fingerprint: BuildFingerprint(1),
        ..batch(vec![patch("after")])
    }
    .to_wire()
    .unwrap();
    rejected.push(json!({"name": "fingerprint", "wire": packed(&fingerprint)}));

    let exact_string = encode(vec![patch("after"), patch(&"x".repeat(MAX_FIELD_BYTES))]);
    let mut over_string = exact_string.clone();
    set_u32(&mut over_string, second + 8, MAX_FIELD_BYTES + 1);
    over_string.push(b'x');
    assert_eq!(
        batch(vec![patch(&"x".repeat(MAX_FIELD_BYTES + 1))]).to_wire(),
        Err(WireError::FieldTooLarge)
    );
    rejected.push(negative(
        "string-over",
        over_string,
        WireError::FieldTooLarge,
    ));

    let exact_bytes = encode(vec![patch("after"), dispatch(MAX_FIELD_BYTES)]);
    let mut over_bytes = exact_bytes.clone();
    let payload_length = over_bytes.len() - MAX_FIELD_BYTES - 4;
    set_u32(&mut over_bytes, payload_length, MAX_FIELD_BYTES + 1);
    over_bytes.push(b'x');
    assert_eq!(
        batch(vec![dispatch(MAX_FIELD_BYTES + 1)]).to_wire(),
        Err(WireError::FieldTooLarge)
    );
    rejected.push(negative("bytes-over", over_bytes, WireError::FieldTooLarge));

    let mut batch_ops = vec![patch("after")];
    batch_ops.extend((0..3).map(|_| dispatch(MAX_FIELD_BYTES)));
    batch_ops.push(dispatch(0));
    let remainder = MAX_BATCH_BYTES - batch(batch_ops.clone()).encoded_len();
    *batch_ops.last_mut().unwrap() = dispatch(remainder);
    let exact_batch = encode(batch_ops.clone());
    assert_eq!(exact_batch.len(), MAX_BATCH_BYTES);
    *batch_ops.last_mut().unwrap() = dispatch(remainder + 1);
    assert_eq!(batch(batch_ops).to_wire(), Err(WireError::BatchTooLarge));
    let mut over_batch = exact_batch.clone();
    over_batch.push(0);
    rejected.push(negative("batch-over", over_batch, WireError::BatchTooLarge));

    let mut effect_ops: Vec<_> = (0..MAX_EFFECTS - 1).map(|_| dispatch(0)).collect();
    effect_ops.push(patch("after"));
    let exact_effects = encode(effect_ops.clone());
    effect_ops.push(dispatch(0));
    assert_eq!(batch(effect_ops).to_wire(), Err(WireError::TooManyEffects));
    let mut over_effects = exact_effects.clone();
    over_effects.extend_from_slice(&encode(vec![dispatch(0)])[HEADER..]);
    set_u32(&mut over_effects, 16, MAX_EFFECTS + 1);
    rejected.push(negative(
        "effects-over",
        over_effects,
        WireError::TooManyEffects,
    ));

    json!({
        "limits": {"batch": MAX_BATCH_BYTES, "field": MAX_FIELD_BYTES, "effects": MAX_EFFECTS},
        "valid": [
            {"name": "small", "wire": packed(&small), "dispatches": 0, "payloadBytes": 0, "textLength": 5},
            {"name": "string-exact", "wire": packed(&exact_string), "dispatches": 0, "payloadBytes": 0, "textLength": MAX_FIELD_BYTES},
            {"name": "bytes-exact", "wire": packed(&exact_bytes), "dispatches": 1, "payloadBytes": MAX_FIELD_BYTES, "textLength": 5},
            {"name": "batch-exact", "wire": packed(&exact_batch), "dispatches": 4, "payloadBytes": 3 * MAX_FIELD_BYTES + remainder, "textLength": 5},
            {"name": "effects-exact", "wire": packed(&exact_effects), "dispatches": MAX_EFFECTS - 1, "payloadBytes": 0, "textLength": 5}
        ],
        "invalid": rejected
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn canonical_boundary_cases_are_compact_and_checked() {
        let value = super::cases();
        assert_eq!(value["valid"].as_array().unwrap().len(), 5);
        assert_eq!(value["invalid"].as_array().unwrap().len(), 16);
        assert!(serde_json::to_vec(&value).unwrap().len() < 200_000);
    }
}
