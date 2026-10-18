use hemx_core::{
    BuildFingerprint, Effect, EffectBatch, IntoEffect, WireError, MAX_EFFECTS, MAX_FIELD_BYTES,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CountingAllocator;

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn record_allocation() {
    if TRACKING.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
    }
}

// SAFETY: Every allocation operation is forwarded unchanged to System. The
// thread-local counters neither allocate nor affect the allocator contract.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn allocations<T>(operation: impl FnOnce() -> T) -> (T, usize) {
    ALLOCATIONS.set(0);
    TRACKING.set(true);
    let result = operation();
    TRACKING.set(false);
    (result, ALLOCATIONS.get())
}

#[test]
fn invalid_batches_are_rejected_before_allocating_output_or_decoded_effects() {
    let valid = vec![Effect::Dispatch {
        event: "saved".into(),
        payload: vec![1, 2, 3],
    }]
    .into_batch(BuildFingerprint(1));
    let (encoded, count) = allocations(|| valid.to_wire());
    assert_eq!(count, 1, "one exact output allocation, no reallocation");
    let bytes = encoded.unwrap();
    let (_, count) = allocations(|| EffectBatch::from_wire(&bytes));
    assert!(
        count > 0,
        "the allocation counter must observe owned decoding"
    );

    let mut incompatible = valid.clone();
    incompatible.abi_version = 1;
    let mut oversized_field = valid.clone();
    oversized_field.ops.push(Effect::Dispatch {
        event: String::new(),
        payload: vec![0; MAX_FIELD_BYTES + 1],
    });
    let oversized_count = vec![valid.ops[0].clone(); MAX_EFFECTS + 1].into_batch(valid.fingerprint);
    let oversized_batch = vec![
        Effect::Dispatch {
            event: String::new(),
            payload: vec![0; MAX_FIELD_BYTES]
        };
        4
    ]
    .into_batch(valid.fingerprint);
    for (batch, expected) in [
        (incompatible, WireError::UnsupportedVersion),
        (oversized_field, WireError::FieldTooLarge),
        (oversized_count, WireError::TooManyEffects),
        (oversized_batch, WireError::BatchTooLarge),
    ] {
        let (result, count) = allocations(|| batch.to_wire());
        assert_eq!((result, count), (Err(expected), 0));
    }

    // A final malformed field must not allocate even the preceding valid effect.
    let mut malformed = bytes.clone();
    malformed[16..20].copy_from_slice(&2_u32.to_le_bytes());
    malformed.push(5); // Dispatch opcode (ABI 3)
    malformed.extend_from_slice(&((MAX_FIELD_BYTES + 1) as u32).to_le_bytes());
    let (result, count) = allocations(|| EffectBatch::from_wire(&malformed));
    assert_eq!((result, count), (Err(WireError::FieldTooLarge), 0));
    let mut truncated = bytes;
    truncated.pop();
    let (result, count) = allocations(|| EffectBatch::from_wire(&truncated));
    assert_eq!((result, count), (Err(WireError::Truncated), 0));
}
