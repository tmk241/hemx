//! Lossless coalescing of declarations.
//!
//! Even repeated declarations can affect retained DOM and unsent form state.
//! Coalescing therefore preserves every operation in order, without inspecting
//! HTML, replacing declarations, or combining row transitions. It saves batch
//! envelopes and transport frames, not browser work. Side effects (Focus,
//! Scroll, Visit, Dispatch) and differing builds remain barriers.

use super::*;
use crate::wire::{MAX_BATCH_BYTES, MAX_EFFECTS};
use core::borrow::Borrow;

/// Coalesce consecutive declaration batches without changing their operations.
///
/// Consecutive batches where each [`folds_into`] its successor form a run; each
/// run becomes the result of [`fold_run`], or stays unfolded when that result
/// would exceed the wire budget.
pub fn fold_batches(batches: impl IntoIterator<Item = EffectBatch>) -> Vec<EffectBatch> {
    let mut folded = Vec::new();
    let mut run: Vec<EffectBatch> = Vec::new();
    for next in batches {
        if !run.last().is_some_and(|last| folds_into(last, &next)) {
            flush(&mut run, &mut folded);
        }
        run.push(next);
    }
    flush(&mut run, &mut folded);
    folded
}

fn flush(run: &mut Vec<EffectBatch>, folded: &mut Vec<EffectBatch>) {
    match (run.len() > 1).then(|| fold_run(run)).flatten() {
        Some(merged) => {
            folded.push(merged);
            run.clear();
        }
        None => folded.append(run),
    }
}

/// Whether two batches may belong to the same coalescing run.
///
/// Both must carry the same ABI and fingerprint and contain only Patch, Rows,
/// or RowsDelta declarations. Targets, patch modes, row keys, and delta hashes
/// do not affect eligibility: all operations remain intact. This does not check
/// the combined wire budget; [`fold_run`] checks it for the whole run.
pub fn folds_into(earlier: &EffectBatch, later: &EffectBatch) -> bool {
    earlier.abi_version == later.abi_version
        && earlier.fingerprint == later.fingerprint
        && earlier.ops.iter().chain(&later.ops).all(|effect| {
            matches!(
                effect,
                Effect::Patch { .. } | Effect::Rows { .. } | Effect::RowsDelta { .. }
            )
        })
}

/// Concatenate every operation of a declaration run in its original order.
///
/// Returns `None` for an empty run, differing ABIs or fingerprints, any side
/// effect, or a total exceeding `MAX_EFFECTS` or `MAX_BATCH_BYTES` (including
/// one envelope). No declarations, row HTML, steps, or hashes are rewritten.
///
/// Inputs are borrowed, so shared batches (for example `Arc<EffectBatch>` from a
/// broadcast) need not be copied individually. The combined size is checked
/// before cloning operations. Individual declarations are not revalidated here;
/// encoding still validates the ABI, fields, and row structure.
pub fn fold_run<B: Borrow<EffectBatch>>(run: &[B]) -> Option<EffectBatch> {
    let first = run.first()?.borrow();
    let mut merged = EffectBatch {
        abi_version: first.abi_version,
        fingerprint: first.fingerprint,
        ops: Vec::new(),
    };
    let envelope = merged.encoded_len();
    let mut bytes = envelope;
    let mut effects = 0usize;
    for batch in run {
        let batch = batch.borrow();
        if !folds_into(first, batch) {
            return None;
        }
        effects = effects.checked_add(batch.ops.len())?;
        bytes = bytes.checked_add(batch.encoded_len().checked_sub(envelope)?)?;
        if effects > MAX_EFFECTS || bytes > MAX_BATCH_BYTES {
            return None;
        }
    }
    merged.ops.reserve(effects);
    for batch in run {
        merged.ops.extend(batch.borrow().ops.iter().cloned());
    }
    Some(merged)
}
