//! The live channel resumes by replay or starts from a snapshot, never leaving a
//! gap or repeating a batch (), and encodes each batch once ().

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::response::IntoResponse;
use futures_util::StreamExt;
use hemx_axum::{Cursor, Live, Resume};
use hemx_core::{
    BuildFingerprint, Effect, EffectBatch, PatchMode, ResourceId, ResourceKind, ResourceRef,
    SafeHtml, WireError,
};

const BUILD: BuildFingerprint = BuildFingerprint(5);

fn patch(id: u32, html: &str) -> Effect {
    Effect::Patch {
        target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, id)),
        mode: PatchMode::Morph,
        html: SafeHtml::trusted(html),
    }
}

/// One SSE event: its id and decoded batch.
fn decode(frame: &[u8]) -> (String, EffectBatch) {
    let text = std::str::from_utf8(frame).unwrap();
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .unwrap_or_else(|| panic!("no {name} in {text:?}"))
            .to_string()
    };
    let (mut bits, mut count, mut bytes) = (0u32, 0, Vec::new());
    for symbol in field("data: ").bytes() {
        let value = match symbol {
            b'A'..=b'Z' => symbol - b'A',
            b'a'..=b'z' => symbol - b'a' + 26,
            b'0'..=b'9' => symbol - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => panic!("not base64url"),
        };
        bits = ((bits << 6) | u32::from(value)) & 0xffff;
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
        }
    }
    (field("id: "), EffectBatch::from_wire(&bytes).unwrap())
}

fn body(
    live: &Live,
    resume: Option<Cursor>,
    snapshot: impl Fn() -> (Cursor, Effect) + Send + Sync + 'static,
) -> impl futures_util::Stream<Item = (String, EffectBatch)> {
    live.sse(Resume(resume), snapshot)
        .into_response()
        .into_body()
        .into_data_stream()
        .map(|frame| decode(&frame.unwrap()))
}

fn counted(
    live: &Live,
    html: &'static str,
) -> (
    Arc<AtomicUsize>,
    impl Fn() -> (Cursor, Effect) + Send + Sync + 'static,
) {
    let calls = Arc::new(AtomicUsize::new(0));
    let (counter, live) = (Arc::clone(&calls), live.clone());
    (calls, move || {
        counter.fetch_add(1, Ordering::SeqCst);
        (live.cursor(), patch(2, html))
    })
}

#[tokio::test]
async fn a_reconnect_within_the_window_replays_exactly_what_it_missed() {
    let live = Live::with_replay(BUILD, 8);
    let seen = live.publish(patch(1, "1")).unwrap();
    live.publish(patch(1, "2")).unwrap();
    let last = live.publish(patch(1, "3")).unwrap();
    let (snapshots, snapshot) = counted(&live, "snapshot");
    let mut events = Box::pin(body(&live, Some(seen), snapshot));
    // The two missed batches share one frame and its last cursor, while every
    // intermediate operation remains present in publication order.
    let (id, batch) = events.next().await.unwrap();
    assert_eq!(id, last.to_string());
    assert_eq!(batch.ops, vec![patch(1, "2"), patch(1, "3")]);
    assert_eq!(
        snapshots.load(Ordering::SeqCst),
        0,
        "replay needs no snapshot"
    );
    let next = live.publish(patch(1, "4")).unwrap();
    let (id, batch) = events.next().await.unwrap();
    assert_eq!((id, batch.ops), (next.to_string(), vec![patch(1, "4")]));
}

#[tokio::test]
async fn replay_over_the_effect_limit_preserves_each_frame_and_cursor() {
    let live = Live::with_replay(BUILD, 8);
    let seen = live.cursor();
    let first_ops = vec![patch(1, "first"); hemx_core::MAX_EFFECTS / 2];
    let second_ops = vec![patch(1, "second"); hemx_core::MAX_EFFECTS / 2 + 1];
    let first = live.publish(first_ops.clone()).unwrap();
    let second = live.publish(second_ops.clone()).unwrap();
    let (snapshots, snapshot) = counted(&live, "snapshot");
    let mut events = Box::pin(body(&live, Some(seen), snapshot));
    let (id, batch) = events.next().await.unwrap();
    assert_eq!((id, batch.ops), (first.to_string(), first_ops));
    let (id, batch) = events.next().await.unwrap();
    assert_eq!((id, batch.ops), (second.to_string(), second_ops));
    assert_eq!(snapshots.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn first_contact_starts_from_a_snapshot_that_covers_concurrent_batches() {
    let live = Live::with_replay(BUILD, 8);
    live.publish(patch(1, "before")).unwrap();
    // A batch published while the snapshot is taken is part of it and must not
    // follow it again.
    let inner = live.clone();
    let mut events = Box::pin(body(&live, None, move || {
        inner.publish(patch(1, "inside")).unwrap();
        (inner.cursor(), patch(2, "snapshot"))
    }));
    let (id, batch) = events.next().await.unwrap();
    assert_eq!(batch.ops, vec![patch(2, "snapshot")]);
    assert_eq!(id, live.cursor().to_string());
    // Even if coalesced into this frame, a repeated inside operation would
    // remain visible before the after operation.
    let after = live.publish(patch(3, "after")).unwrap();
    let (id, batch) = events.next().await.unwrap();
    assert_eq!(
        batch.ops,
        vec![patch(3, "after")],
        "the inside batch is not repeated"
    );
    assert_eq!(id, after.to_string());
}

#[tokio::test]
async fn a_cursor_from_a_restart_or_beyond_the_window_gets_a_snapshot() {
    let live = Live::with_replay(BUILD, 2);
    let old = live.publish(patch(1, "1")).unwrap();
    for n in 2..=5 {
        live.publish(patch(1, &n.to_string())).unwrap();
    }
    let previous_process = Live::with_replay(BUILD, 2);
    std::thread::sleep(std::time::Duration::from_millis(1)); // A distinct epoch.
    let restarted = Live::with_replay(BUILD, 2);
    for resume in [old, previous_process.cursor()] {
        let (snapshots, snapshot) = counted(&live, "snapshot");
        let (_, batch) = Box::pin(body(&live, Some(resume), snapshot))
            .next()
            .await
            .unwrap();
        assert_eq!(batch.ops, vec![patch(2, "snapshot")], "{resume}");
        assert_eq!(snapshots.load(Ordering::SeqCst), 1);
    }
    assert_ne!(restarted.cursor(), previous_process.cursor());
}

#[tokio::test]
async fn a_connection_that_falls_behind_starts_again_from_a_snapshot() {
    let live = Live::with_replay(BUILD, 2);
    let (snapshots, snapshot) = counted(&live, "caught up");
    let mut events = Box::pin(body(&live, Some(live.cursor()), snapshot));
    // Nothing missed yet: no snapshot. Then more than the window, unread.
    for n in 0..5 {
        live.publish(patch(1, &n.to_string())).unwrap();
    }
    let (id, batch) = events.next().await.unwrap();
    assert_eq!(batch.ops, vec![patch(2, "caught up")]);
    assert_eq!(id, live.cursor().to_string());
    assert_eq!(snapshots.load(Ordering::SeqCst), 1);
}

#[test]
fn publishing_rejects_what_the_wire_rejects_without_taking_a_position() {
    let live = Live::new(BUILD);
    let before = live.cursor();
    let oversized = patch(1, &"x".repeat(hemx_core::MAX_FIELD_BYTES + 1));
    assert_eq!(live.publish(oversized), Err(WireError::FieldTooLarge));
    assert_eq!(live.cursor(), before);
}

#[tokio::test]
async fn resume_prefers_last_event_id_over_the_page_cursor() {
    let live = Live::new(BUILD);
    let page = live.cursor();
    let reconnect = live.publish(patch(1, "x")).unwrap();
    let request = axum::http::Request::builder()
        .uri(format!("/events?after={page}"))
        .header("last-event-id", reconnect.to_string())
        .body(())
        .unwrap();
    let (mut parts, ()) = request.into_parts();
    let Resume(cursor) = Resume::from_request_parts(&mut parts, &()).await.unwrap();
    assert_eq!(cursor, Some(reconnect));
    let request = axum::http::Request::builder()
        .uri(format!("/events?after={page}"))
        .body(())
        .unwrap();
    let (mut parts, ()) = request.into_parts();
    let Resume(cursor) = Resume::from_request_parts(&mut parts, &()).await.unwrap();
    assert_eq!(cursor, Some(page));
    let request = axum::http::Request::builder()
        .uri("/events?after=nonsense")
        .body(())
        .unwrap();
    let (mut parts, ()) = request.into_parts();
    let Resume(cursor) = Resume::from_request_parts(&mut parts, &()).await.unwrap();
    assert_eq!(cursor, None, "unreadable cursors start from a snapshot");
}
