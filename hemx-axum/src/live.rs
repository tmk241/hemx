//! A live channel that never leaves a client with a gap (, ).
//!
//! Every published batch gets the next [`Cursor`]. A connection resumes from the
//! cursor the browser last saw (`Last-Event-ID` on a reconnect, or the cursor the
//! page was rendered at) by replaying what it missed; when that is not possible
//! (first contact, a server restart, a gap older than the replay window, or a
//! lagging connection) it starts from a snapshot of the current state instead.
//! Each batch is encoded once when it is published and that text is shared by
//! every connection that sends it unfolded.

use std::borrow::Borrow;
use std::collections::VecDeque;
use std::convert::Infallible;
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::{stream, Stream, StreamExt};
use hemx_core::{BuildFingerprint, EffectBatch, IntoEffect, WireError};
use tokio::sync::broadcast;

use crate::{encode_sse_batch, Folding, Frame, HEMX_SSE_EVENT};

/// The position of a client in a [`Live`] channel's sequence of batches.
///
/// It names the process (`epoch`) as well as the position, so a cursor from
/// before a restart never matches. Its text form, `epoch.seq`, is what the
/// browser sends back as `Last-Event-ID`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cursor {
    epoch: u64,
    seq: u64,
}

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:x}.{}", self.epoch, self.seq)
    }
}

impl FromStr for Cursor {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, ()> {
        let (epoch, seq) = text.split_once('.').ok_or(())?;
        Ok(Self {
            epoch: u64::from_str_radix(epoch, 16).map_err(|_| ())?,
            seq: seq.parse().map_err(|_| ())?,
        })
    }
}

/// Where a connection wants to resume: the `Last-Event-ID` header of an
/// `EventSource` reconnect, else the `after` query parameter a page put into its
/// `data-hemx-sse` URL. Anything unreadable means "start from a snapshot".
#[derive(Clone, Copy, Debug, Default)]
pub struct Resume(pub Option<Cursor>);

impl<S: Send + Sync> FromRequestParts<S> for Resume {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Infallible> {
        let header = parts
            .headers
            .get("last-event-id")
            .and_then(|value| value.to_str().ok())
            .and_then(|text| text.parse().ok());
        let query = || {
            parts.uri.query()?.split('&').find_map(|pair| {
                let value = pair.strip_prefix("after=")?;
                value.parse().ok()
            })
        };
        Ok(Self(header.or_else(query)))
    }
}

/// A batch as published: its position, the batch, and its encoded event data.
#[derive(Clone)]
struct Published {
    seq: u64,
    batch: Arc<EffectBatch>,
    data: Arc<str>,
}

/// What a live connection sends: a published batch or its starting snapshot.
#[derive(Clone)]
enum Item {
    Published(Published),
    Snapshot {
        seq: u64,
        batch: Arc<EffectBatch>,
        data: Arc<str>,
    },
}

impl Item {
    fn seq(&self) -> u64 {
        match self {
            Item::Published(published) => published.seq,
            Item::Snapshot { seq, .. } => *seq,
        }
    }

    fn data(&self) -> &Arc<str> {
        match self {
            Item::Published(published) => &published.data,
            Item::Snapshot { data, .. } => data,
        }
    }
}

impl Borrow<EffectBatch> for Item {
    fn borrow(&self) -> &EffectBatch {
        match self {
            Item::Published(published) => &published.batch,
            Item::Snapshot { batch, .. } => batch,
        }
    }
}

struct Sequence {
    seq: u64,
    replay: VecDeque<Published>,
}

struct Shared {
    epoch: u64,
    fingerprint: BuildFingerprint,
    capacity: usize,
    sequence: Mutex<Sequence>,
    sender: broadcast::Sender<Published>,
}

/// One ordered stream of batches for any number of SSE connections.
///
/// Publish while holding the lock that guards the state the batches describe,
/// and read [`Live::cursor`] under that same lock when rendering a page or a
/// snapshot: then a cursor and the state it is read with always agree, and no
/// connection misses or repeats a batch.
#[derive(Clone)]
pub struct Live {
    shared: Arc<Shared>,
}

impl Live {
    /// A channel that can replay the last 1,024 batches to a reconnecting client.
    pub fn new(fingerprint: BuildFingerprint) -> Self {
        Self::with_replay(fingerprint, 1_024)
    }

    /// A channel that can replay the last `capacity` batches (at least one).
    pub fn with_replay(fingerprint: BuildFingerprint, capacity: usize) -> Self {
        let capacity = capacity.max(1);
        // Distinct per process start, so cursors from before a restart never match.
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos() as u64);
        Self {
            shared: Arc::new(Shared {
                epoch,
                fingerprint,
                capacity,
                sequence: Mutex::new(Sequence {
                    seq: 0,
                    replay: VecDeque::with_capacity(capacity),
                }),
                sender: broadcast::channel(capacity).0,
            }),
        }
    }

    /// The position after the last published batch.
    pub fn cursor(&self) -> Cursor {
        let seq = self.lock().seq;
        Cursor {
            epoch: self.shared.epoch,
            seq,
        }
    }

    /// Encode and publish one batch to every connection, returning its position.
    /// A batch the wire format rejects is not published and takes no position.
    pub fn publish(&self, effects: impl IntoEffect) -> Result<Cursor, WireError> {
        let batch = effects.into_batch(self.shared.fingerprint);
        let data: Arc<str> = encode_sse_batch(&batch)?.into();
        let mut sequence = self.lock();
        sequence.seq += 1;
        let published = Published {
            seq: sequence.seq,
            batch: Arc::new(batch),
            data,
        };
        if sequence.replay.len() == self.shared.capacity {
            sequence.replay.pop_front();
        }
        sequence.replay.push_back(published.clone());
        // Sending under the lock keeps the broadcast in sequence order.
        let _ = self.shared.sender.send(published);
        Ok(Cursor {
            epoch: self.shared.epoch,
            seq: sequence.seq,
        })
    }

    /// Serve one SSE connection resuming from `resume`.
    ///
    /// `snapshot` returns the current state as effects together with the
    /// [`Live::cursor`] read under the same lock as that state. It runs when the
    /// connection cannot be resumed by replay, and again after the connection
    /// fell behind the replay window. Its batch must fit the wire budgets; for
    /// state that may not, return a `Visit` of the page instead.
    pub fn sse<F, E>(
        &self,
        resume: Resume,
        snapshot: F,
    ) -> Sse<impl Stream<Item = Result<Event, axum::BoxError>> + Send>
    where
        F: Fn() -> (Cursor, E) + Send + Sync + 'static,
        E: IntoEffect + 'static,
    {
        let live = self.clone();
        let epoch = self.shared.epoch;
        let snapshot = Arc::new(snapshot);
        let (replay, receiver) = live.subscribe(resume.0);
        let items = stream::unfold(
            Connection {
                live,
                snapshot,
                receiver,
                pending: replay.map(|items| items.into_iter().map(Item::Published).collect()),
                floor: 0,
            },
            Connection::next,
        );
        Sse::new(Folding::new(items).map(move |frame| {
            let frame: Frame<Item> = frame?;
            let (last, data) = match &frame {
                Frame::Received(item) => (item, Arc::clone(item.data())),
                Frame::Folded(batch, last) => (last, encode_sse_batch(batch)?.into()),
            };
            let id = Cursor {
                epoch,
                seq: last.seq(),
            };
            Ok::<_, axum::BoxError>(event(id, &data))
        }))
        .keep_alive(KeepAlive::default())
    }

    /// Subscribe, and collect the replay for `resume` under the same lock, so the
    /// replay and the subscription neither overlap nor leave a gap. `None` means
    /// the connection must start from a snapshot.
    fn subscribe(
        &self,
        resume: Option<Cursor>,
    ) -> (Option<Vec<Published>>, broadcast::Receiver<Published>) {
        let sequence = self.lock();
        let receiver = self.shared.sender.subscribe();
        let replay = resume
            .filter(|cursor| cursor.epoch == self.shared.epoch && cursor.seq <= sequence.seq)
            .filter(|cursor| {
                cursor.seq == sequence.seq
                    || sequence
                        .replay
                        .front()
                        .is_some_and(|oldest| oldest.seq <= cursor.seq + 1)
            })
            .map(|cursor| {
                sequence
                    .replay
                    .iter()
                    .filter(|published| published.seq > cursor.seq)
                    .cloned()
                    .collect()
            });
        (replay, receiver)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Sequence> {
        self.shared
            .sequence
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn event(id: Cursor, data: &str) -> Event {
    Event::default()
        .event(HEMX_SSE_EVENT)
        .id(id.to_string())
        .data(data)
}

/// The state of one connection: what is still to be sent, and the position up
/// to which a snapshot already covers published batches.
struct Connection<F> {
    live: Live,
    snapshot: Arc<F>,
    receiver: broadcast::Receiver<Published>,
    /// `None` until the connection has its starting point.
    pending: Option<VecDeque<Item>>,
    /// Published batches at or below this position are inside a snapshot.
    floor: u64,
}

impl<F, E> Connection<F>
where
    F: Fn() -> (Cursor, E) + Send + Sync + 'static,
    E: IntoEffect,
{
    async fn next(mut self) -> Option<(Result<Item, axum::BoxError>, Self)> {
        loop {
            match self.pending.as_mut() {
                None => match self.start() {
                    Ok(item) => return Some((Ok(item), self)),
                    Err(error) => {
                        self.pending = Some(VecDeque::new());
                        return Some((Err(error.into()), self));
                    }
                },
                Some(pending) => {
                    if let Some(item) = pending.pop_front() {
                        return Some((Ok(item), self));
                    }
                }
            }
            match self.receiver.recv().await {
                Ok(published) if published.seq <= self.floor => continue,
                Ok(published) => return Some((Ok(Item::Published(published)), self)),
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Too far behind to replay: start again from a snapshot.
                    self.receiver = self.live.subscribe(None).1;
                    self.pending = None;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Take a snapshot after subscribing: batches published in between are
    /// covered by it and skipped, later ones follow it.
    fn start(&mut self) -> Result<Item, WireError> {
        let (cursor, effects) = (self.snapshot)();
        let batch = effects.into_batch(self.live.shared.fingerprint);
        let data: Arc<str> = encode_sse_batch(&batch)?.into();
        self.floor = cursor.seq;
        self.pending = Some(VecDeque::new());
        Ok(Item::Snapshot {
            seq: cursor.seq,
            batch: Arc::new(batch),
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hemx_core::Effect;

    #[test]
    fn a_batch_is_encoded_once_for_every_connection() {
        let live = Live::new(BuildFingerprint(1));
        let (_, mut first) = live.subscribe(None);
        let (_, mut second) = live.subscribe(None);
        live.publish(Effect::Dispatch {
            event: "saved".into(),
            payload: Vec::new(),
        })
        .unwrap();
        let (a, b) = (first.try_recv().unwrap(), second.try_recv().unwrap());
        // Both connections send the text encoded at publication.
        assert!(Arc::ptr_eq(&a.data, &b.data));
        assert!(Arc::ptr_eq(Item::Published(a.clone()).data(), &a.data));
    }
}
