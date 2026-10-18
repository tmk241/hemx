//! The SSE helper folds batches that wait while the client cannot accept more.

use std::convert::Infallible;

use axum::response::IntoResponse;
use futures_util::{stream, StreamExt};
use hemx_axum::sse;
use hemx_core::{
    BuildFingerprint, Effect, EffectBatch, IntoEffect, PatchMode, ResourceId, ResourceKind,
    ResourceRef, SafeHtml,
};

fn patch(id: u32, html: &str) -> Effect {
    Effect::Patch {
        target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, id)),
        mode: PatchMode::Morph,
        html: SafeHtml::trusted(html),
    }
}

fn batch(ops: Vec<Effect>) -> EffectBatch {
    ops.into_batch(BuildFingerprint(5))
}

/// Decode one `event: hemx` frame back to its canonical batch.
fn decode(frame: &[u8]) -> EffectBatch {
    let text = std::str::from_utf8(frame).unwrap();
    let data = text
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .unwrap();
    let mut bits = 0u32;
    let mut count = 0;
    let mut bytes = Vec::new();
    for symbol in data.bytes() {
        let value = match symbol {
            b'A'..=b'Z' => symbol - b'A',
            b'a'..=b'z' => symbol - b'a' + 26,
            b'0'..=b'9' => symbol - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => panic!("not base64url: {symbol}"),
        };
        bits = (bits << 6) | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
        }
    }
    EffectBatch::from_wire(&bytes).unwrap()
}

async fn frames(batches: Vec<Result<EffectBatch, Infallible>>) -> Vec<EffectBatch> {
    let body = sse(stream::iter(batches))
        .into_response()
        .into_body()
        .into_data_stream();
    body.map(|frame| decode(&frame.unwrap())).collect().await
}

#[tokio::test]
async fn waiting_updates_reach_the_client_in_one_frame_without_losing_operations() {
    let operations: Vec<_> = (0..1000).map(|n| patch(1, &n.to_string())).collect();
    let updates = operations
        .iter()
        .map(|op| Ok(batch(vec![op.clone()])))
        .collect();
    let sent = frames(updates).await;
    assert_eq!(sent, vec![batch(operations)]);
}

#[tokio::test]
async fn side_effects_are_never_folded_away() {
    let focus = Effect::Focus {
        target: ResourceRef::unscoped(ResourceId::new(ResourceKind::Slot, 1)),
        prevent_scroll: false,
    };
    let sent = frames(vec![
        Ok(batch(vec![patch(1, "a")])),
        Ok(batch(vec![focus.clone()])),
        Ok(batch(vec![patch(1, "b")])),
    ])
    .await;
    assert_eq!(
        sent,
        vec![
            batch(vec![patch(1, "a")]),
            batch(vec![focus]),
            batch(vec![patch(1, "b")]),
        ]
    );
}

#[tokio::test]
async fn an_upstream_error_follows_the_batches_that_preceded_it() {
    let failing = stream::iter(vec![
        Ok(batch(vec![patch(1, "a")])),
        Ok(batch(vec![patch(1, "b")])),
        Err(std::io::Error::other("upstream failed")),
    ]);
    let mut body = sse(failing).into_response().into_body().into_data_stream();
    assert_eq!(
        decode(&body.next().await.unwrap().unwrap()),
        batch(vec![patch(1, "a"), patch(1, "b")])
    );
    assert!(body.next().await.unwrap().is_err());
}

#[tokio::test]
async fn a_client_that_keeps_up_receives_every_batch() {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<EffectBatch>();
    let live = stream::unfold(receiver, |mut receiver| async move {
        receiver
            .recv()
            .await
            .map(|batch| (Ok::<_, Infallible>(batch), receiver))
    });
    let mut body = sse(live).into_response().into_body().into_data_stream();
    for n in 0..3 {
        sender.send(batch(vec![patch(1, &n.to_string())])).unwrap();
        let frame = body.next().await.unwrap().unwrap();
        assert_eq!(decode(&frame), batch(vec![patch(1, &n.to_string())]));
    }
}

#[tokio::test]
async fn shared_batches_from_a_broadcast_fold_like_owned_ones() {
    // One Arc per update, as a broadcast hands to every connection.
    let shared: Vec<Result<std::sync::Arc<EffectBatch>, Infallible>> = (0..100)
        .map(|n| Ok(std::sync::Arc::new(batch(vec![patch(1, &n.to_string())]))))
        .collect();
    let body = sse(stream::iter(shared))
        .into_response()
        .into_body()
        .into_data_stream();
    let sent: Vec<EffectBatch> = body.map(|frame| decode(&frame.unwrap())).collect().await;
    assert_eq!(
        sent,
        vec![batch((0..100).map(|n| patch(1, &n.to_string())).collect())]
    );
}

#[tokio::test]
async fn runs_over_wire_limits_keep_their_original_frames() {
    for waiting in [
        vec![
            batch(vec![patch(1, "a"); hemx_core::MAX_EFFECTS / 2]),
            batch(vec![patch(1, "b"); hemx_core::MAX_EFFECTS / 2]),
            batch(vec![patch(1, "c")]),
        ],
        vec![
            batch(vec![patch(1, &"a".repeat(200_000)); 3]),
            batch(vec![patch(1, &"b".repeat(200_000)); 3]),
        ],
    ] {
        let sent = frames(waiting.iter().cloned().map(Ok).collect()).await;
        assert_eq!(sent, waiting);
    }
}
