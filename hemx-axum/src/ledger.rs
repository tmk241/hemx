//! At-most-once enhanced actions.
//!
//! The browser runtime sends every enhanced POST with a fresh random token and
//! reuses it only for its one automatic resend after a lost or late response.
//! A ledger remembers, per token, which action ran it and its first response.
//! A resend then receives that response instead of running the handler again.
//!
//! The ledger is application state: [`MemoryLedger`] suits one process; a
//! deployment with several processes implements [`Ledger`] on shared storage.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{to_bytes, Body, Bytes};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

/// Request header carrying an enhanced submission's token.
pub const HEMX_TOKEN_HEADER: &str = "x-hemx-token";
/// Response header declaring that this action deduplicates tokens.
pub const HEMX_LEDGER_HEADER: &str = "x-hemx-ledger";

/// One enhanced submission's token: 16 to 64 ASCII letters, digits, `-` or `_`.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Token(String);

impl Token {
    /// A well-formed token, or `None`.
    pub fn parse(value: &str) -> Option<Self> {
        let valid = (16..=64).contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        valid.then(|| Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A complete response, kept so a resend receives exactly the same one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Recorded {
    pub status: StatusCode,
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub body: Bytes,
}

impl Recorded {
    /// Read a response completely. Bodies are effect batches or short messages.
    ///
    /// Returns the body-read error rather than recording an incomplete response.
    pub async fn read(response: Response) -> Result<Self, axum::Error> {
        let (parts, body) = response.into_parts();
        let body = to_bytes(body, usize::MAX).await?;
        Ok(Self {
            status: parts.status,
            headers: parts
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            body,
        })
    }

    fn size(&self) -> usize {
        self.body.len()
            + self
                .headers
                .iter()
                .map(|(name, value)| name.as_str().len() + value.len())
                .sum::<usize>()
    }
}

impl IntoResponse for Recorded {
    fn into_response(self) -> Response {
        let mut response = Response::new(Body::from(self.body));
        *response.status_mut() = self.status;
        for (name, value) in self.headers {
            response.headers_mut().append(name, value);
        }
        response
    }
}

/// What a ledger knows about a token when a request arrives.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Claim {
    /// Unknown token, now recorded as running: dispatch it.
    New,
    /// Its first request has not finished: do not dispatch again.
    Running,
    /// Its first request finished with this response.
    Done(Recorded),
    /// The token was used for a different action.
    OtherAction,
    /// The ledger cannot take another token now.
    Unavailable,
}

/// Remembers enhanced submissions by token. Implement on shared storage when
/// several processes serve the same actions.
pub trait Ledger: Send + Sync + 'static {
    /// Look up `token` for the action `handle`, claiming it when unknown.
    fn claim(&self, token: &Token, handle: u32) -> impl Future<Output = Claim> + Send;
    /// Store the first complete response of a claimed token and return the
    /// response actually kept. The first caller receives exactly that response,
    /// so it never differs from what a resend of the token receives.
    fn record(&self, token: &Token, response: Recorded) -> impl Future<Output = Recorded> + Send;
}

/// Bounds of a [`MemoryLedger`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LedgerLimits {
    /// Most tokens remembered at once, running or finished.
    pub entries: usize,
    /// Most response bytes kept, including a reserve for every running token.
    pub bytes: usize,
    /// How long a finished token is remembered after it started.
    pub age: Duration,
}

/// A bounded in-process ledger for one server process.
///
/// A token is remembered for `age` after it started; within that time neither
/// `entries` nor `bytes` pressure forgets it. When remembering another token
/// would exceed either bound, the new submission is refused (503) before its
/// handler runs. Every running token reserves room for a short 410 answer;
/// a response that does not fit the remaining bytes is kept, and returned to
/// the first caller too, as that 410. Keep `age` above the longest time a
/// browser could resend after its first attempt (the action's deadline).
#[derive(Clone)]
pub struct MemoryLedger {
    limits: LedgerLimits,
    state: Arc<Mutex<Entries>>,
}

#[derive(Default)]
struct Entries {
    map: HashMap<Token, Entry>,
    order: VecDeque<Token>,
    /// Bytes of kept responses.
    bytes: usize,
    /// Tokens claimed but not yet recorded; each reserves `gone().size()`.
    running: usize,
}

struct Entry {
    handle: u32,
    started: Instant,
    response: Option<Recorded>,
}

impl MemoryLedger {
    pub fn new(limits: LedgerLimits) -> Self {
        Self {
            limits,
            state: Arc::default(),
        }
    }

    /// Number of remembered tokens and kept response bytes.
    pub fn usage(&self) -> (usize, usize) {
        let state = self.state.lock().expect("ledger lock poisoned");
        (state.map.len(), state.bytes)
    }
}

impl Entries {
    /// Forget finished tokens older than `age`. Running tokens stay.
    fn forget_expired(&mut self, age: Duration) {
        let mut kept = VecDeque::with_capacity(self.order.len());
        while let Some(token) = self.order.pop_front() {
            let expired = self
                .map
                .get(&token)
                .is_some_and(|entry| entry.response.is_some() && entry.started.elapsed() > age);
            if expired {
                if let Some(Entry {
                    response: Some(response),
                    ..
                }) = self.map.remove(&token)
                {
                    self.bytes -= response.size();
                }
            } else {
                kept.push_back(token);
            }
        }
        self.order = kept;
    }

    /// Bytes promised to kept responses and to running tokens' 410 answers.
    fn committed(&self) -> usize {
        self.bytes + self.running * gone().size()
    }
}

impl Ledger for MemoryLedger {
    async fn claim(&self, token: &Token, handle: u32) -> Claim {
        let limits = self.limits;
        let mut state = self.state.lock().expect("ledger lock poisoned");
        state.forget_expired(limits.age);
        if let Some(entry) = state.map.get(token) {
            return match (&entry.response, entry.handle == handle) {
                (_, false) => Claim::OtherAction,
                (None, true) => Claim::Running,
                (Some(response), true) => Claim::Done(response.clone()),
            };
        }
        let fits =
            state.map.len() < limits.entries && state.committed() + gone().size() <= limits.bytes;
        if !fits {
            return Claim::Unavailable;
        }
        state.map.insert(
            token.clone(),
            Entry {
                handle,
                started: Instant::now(),
                response: None,
            },
        );
        state.order.push_back(token.clone());
        state.running += 1;
        Claim::New
    }

    async fn record(&self, token: &Token, response: Recorded) -> Recorded {
        let mut state = self.state.lock().expect("ledger lock poisoned");
        let state = &mut *state;
        match state.map.get(token) {
            // Never claimed here: nothing to remember, nothing to replay.
            None => return response,
            Some(Entry {
                response: Some(kept),
                ..
            }) => return kept.clone(),
            Some(_) => {}
        }
        // Release this token's reserve, then keep the response if it fits.
        state.running -= 1;
        let free = self.limits.bytes.saturating_sub(state.committed());
        let kept = if response.size() <= free {
            response
        } else {
            gone()
        };
        state.bytes += kept.size();
        if let Some(entry) = state.map.get_mut(token) {
            entry.response = Some(kept.clone());
        }
        kept
    }
}

/// The answer kept when a result is too large to replay.
fn gone() -> Recorded {
    const MESSAGE: &str = "the result of this submission is too large to repeat; reload the page";
    Recorded {
        status: StatusCode::GONE,
        headers: vec![(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )],
        body: Bytes::from_static(MESSAGE.as_bytes()),
    }
}

/// Answers for tokens the ledger will not dispatch.
pub(crate) fn refusal(claim: &Claim) -> Option<Response> {
    let (status, message) = match claim {
        Claim::Running => (
            StatusCode::TOO_EARLY,
            "this submission is still being processed",
        ),
        Claim::OtherAction => (
            StatusCode::BAD_REQUEST,
            "this submission token belongs to another action",
        ),
        Claim::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "too many submissions are in progress; try again",
        ),
        Claim::New | Claim::Done(_) => return None,
    };
    Some((status, message).into_response())
}
