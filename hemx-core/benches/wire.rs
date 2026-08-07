//! Repeatable wire-encoding baseline: `cargo bench -p hemx-core --bench wire`.

use hemx_core::{
    BuildFingerprint, Effect, EffectBatch, Payload, ResourceId, ResourceKind, ResourceRef,
    EFFECT_BATCH_ABI_VERSION,
};
use std::hint::black_box;
use std::time::Instant;

const ITERATIONS: u32 = 100_000;
const ROUNDS: usize = 7;

fn representative_batch() -> EffectBatch {
    let target = ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 42));
    let ops = (0..50)
        .map(|index| Effect::Put {
            target: target.clone(),
            payload: Payload::Text(format!("item-{index}-{}", "x".repeat(64))),
        })
        .collect();
    EffectBatch {
        abi_version: EFFECT_BATCH_ABI_VERSION,
        fingerprint: BuildFingerprint(7),
        ops,
    }
}

fn main() {
    let batch = representative_batch();
    for _ in 0..10_000 {
        black_box(batch.to_wire());
    }

    let mut nanos_per_batch = [0_u128; ROUNDS];
    let mut checksum = 0_usize;
    for round in &mut nanos_per_batch {
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            let bytes = black_box(&batch).to_wire();
            checksum = checksum.wrapping_add(black_box(bytes.len()));
        }
        *round = start.elapsed().as_nanos() / u128::from(ITERATIONS);
    }
    nanos_per_batch.sort_unstable();

    println!(
        "median_ns_per_batch={} wire_len={} checksum={checksum}",
        nanos_per_batch[ROUNDS / 2],
        batch.encoded_len(),
    );
}
