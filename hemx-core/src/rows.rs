//! Row orders as named states.
//!
//! A collection's state is its key order. An [`order_hash`] names that state,
//! so a [`Effect::RowsDelta`] can say which state it starts from and which it
//! produces. A client applies a delta only at its starting state, which makes a
//! positional change as safe and idempotent as a full declaration while its
//! size stays proportional to what changed.

use super::*;
use alloc::collections::BTreeMap;
use serde::{Deserialize, Serialize};

/// One step of a [`Effect::RowsDelta`], applied in sequence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RowStep {
    /// Remove the live row with this key.
    Remove(String),
    /// Place the row with `key` immediately before the row `before`, or at the
    /// end when `before` is `None`. A live row moves; otherwise the delta must
    /// supply its HTML and the row is created.
    Place { key: String, before: Option<String> },
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Name a key order: FNV-1a 64 over each key's UTF-8 bytes followed by `0xFF`,
/// a byte UTF-8 never contains. The browser runtime computes the same value
/// from the live rows. This detects a client in another state; it is not a
/// cryptographic commitment.
pub fn order_hash<K: AsRef<str>>(keys: impl IntoIterator<Item = K>) -> u64 {
    let mut hash = FNV_OFFSET;
    for key in keys {
        for byte in key.as_ref().bytes().chain(core::iter::once(0xff)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    hash
}

/// Steps that turn `before` into `after`: removals in `before` order, then
/// placements from the end of `after` towards its start, each before its
/// already placed successor. Rows on a longest increasing run of retained
/// positions stay where they are, so only the rows that must move do.
///
/// Both orders must have unique keys; with duplicates the result is
/// meaningless.
///
/// A shared prefix and suffix are skipped by comparison alone, so the usual
/// change (a row added, removed or moved in one place) costs one pass over
/// the keys without building any index of them.
pub fn row_steps<K: AsRef<str>>(before: &[K], after: &[K]) -> Vec<RowStep> {
    let same = |(left, right): (&K, &K)| left.as_ref() == right.as_ref();
    let prefix = before
        .iter()
        .zip(after)
        .take_while(|pair| same(*pair))
        .count();
    let room = before.len().min(after.len()) - prefix;
    let suffix = before
        .iter()
        .rev()
        .zip(after.iter().rev())
        .take(room)
        .take_while(|pair| same(*pair))
        .count();
    // Unique keys: nothing in either middle occurs in the shared ends.
    let old = &before[prefix..before.len() - suffix];
    let new = prefix..after.len() - suffix;
    let position: BTreeMap<&str, usize> = old
        .iter()
        .enumerate()
        .map(|(index, key)| (key.as_ref(), index))
        .collect();
    let retained: BTreeMap<&str, ()> = after[new.clone()]
        .iter()
        .map(|key| (key.as_ref(), ()))
        .collect();
    let mut steps: Vec<RowStep> = old
        .iter()
        .filter(|key| !retained.contains_key(key.as_ref()))
        .map(|key| RowStep::Remove(key.as_ref().into()))
        .collect();
    // Positions in `before` of the retained keys, in `after` order.
    let indices: Vec<Option<usize>> = after[new.clone()]
        .iter()
        .map(|key| position.get(key.as_ref()).copied())
        .collect();
    let stable = longest_increasing(&indices);
    for index in new.rev() {
        if stable[index - prefix] {
            continue;
        }
        steps.push(RowStep::Place {
            key: after[index].as_ref().into(),
            before: after.get(index + 1).map(|next| next.as_ref().into()),
        });
    }
    steps
}

/// Mark the entries on one longest strictly increasing subsequence of the
/// present values (patience sorting, O(n log n)).
fn longest_increasing(values: &[Option<usize>]) -> Vec<bool> {
    let mut tails: Vec<usize> = Vec::new(); // index into `values` of each run's tail
    let mut previous: Vec<Option<usize>> = alloc::vec![None; values.len()];
    for (index, value) in values.iter().enumerate() {
        let Some(value) = *value else { continue };
        let at = tails.partition_point(|&tail| values[tail].is_some_and(|tail| tail < value));
        previous[index] = at.checked_sub(1).map(|before| tails[before]);
        if at == tails.len() {
            tails.push(index);
        } else {
            tails[at] = index;
        }
    }
    let mut stable = alloc::vec![false; values.len()];
    let mut cursor = tails.last().copied();
    while let Some(index) = cursor {
        stable[index] = true;
        cursor = previous[index];
    }
    stable
}

/// Apply steps to a key order, as a client at `before` would; `None` when a
/// step does not apply. The tests check every generated delta with it.
#[cfg(test)]
fn apply_steps(order: &mut Vec<String>, steps: &[RowStep]) -> Option<()> {
    for step in steps {
        match step {
            RowStep::Remove(key) => {
                let at = order.iter().position(|live| live == key)?;
                order.remove(at);
            }
            RowStep::Place { key, before } => {
                if let Some(at) = order.iter().position(|live| live == key) {
                    order.remove(at);
                }
                let at = match before {
                    Some(anchor) => order.iter().position(|live| live == anchor)?,
                    None => order.len(),
                };
                order.insert(at, key.clone());
            }
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    fn keys(text: &str) -> Vec<String> {
        text.split_whitespace().map(ToString::to_string).collect()
    }

    #[test]
    fn steps_turn_every_order_into_every_other() {
        let orders = [
            "",
            "a",
            "a b c",
            "c b a",
            "b c a",
            "a c",
            "d a b",
            "e d c b a",
            "a b c d e f",
        ];
        for before in orders {
            for after in orders {
                let (before, after) = (keys(before), keys(after));
                let steps = row_steps(&before, &after);
                let mut order = before.clone();
                assert_eq!(apply_steps(&mut order, &steps), Some(()));
                assert_eq!(order, after, "{before:?} -> {after:?} via {steps:?}");
            }
        }
    }

    #[test]
    fn only_rows_that_must_move_are_placed() {
        // One appended row: one placement, nothing else touched.
        let steps = row_steps(&keys("a b c"), &keys("a b c d"));
        assert_eq!(
            steps,
            [RowStep::Place {
                key: "d".into(),
                before: None
            }]
        );
        // Moving the first row to the end moves just that row.
        assert_eq!(row_steps(&keys("a b c"), &keys("b c a")).len(), 1);
        // A removal is one step.
        assert_eq!(
            row_steps(&keys("a b c"), &keys("a c")),
            [RowStep::Remove("b".into())]
        );
    }

    #[test]
    fn order_hash_names_the_order_not_just_the_members() {
        assert_ne!(order_hash(["a", "b"]), order_hash(["b", "a"]));
        assert_ne!(order_hash(["ab"]), order_hash(["a", "b"]));
        assert_eq!(order_hash(Vec::<String>::new()), FNV_OFFSET);
        // Fixed vectors, computed independently and shared with the browser runtime.
        assert_eq!(order_hash(["1", "2"]), 0x40e5_6fed_1ecb_4a6a);
        assert_eq!(order_hash(["todo-7", "\u{e4}"]), 0x4eab_7f6f_ad64_e622);
    }
}
