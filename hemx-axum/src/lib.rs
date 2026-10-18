use axum::body::{Body, Bytes};
pub use axum::extract::State;
use axum::extract::{FromRequest, FromRequestParts, Multipart};
use axum::http::{header, request::Parts, HeaderMap, HeaderValue, Request, Response, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use futures_util::{Stream, StreamExt};
use hemx_core::{BuildFingerprint, EffectBatch, FromForm, Handle, IntoEffect, SafeHtml, WireError};
use std::borrow::Borrow;
use std::collections::{BTreeMap, VecDeque};
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;

mod forms;
mod live;

pub use forms::{Form, FromInteractionForm, InteractionFile, InteractionForm, InteractionRequest};
pub use live::{Cursor, Live, Resume};

pub const HEMX_PARTIAL_HEADER: &str = "x-hemx-partial";
pub const HEMX_FINGERPRINT_HEADER: &str = "x-hemx-fingerprint";
pub const HEMX_TITLE_HEADER: &str = "x-hemx-title";
pub const HEMX_CONTENT_TYPE: &str = "application/hemx";
pub const HEMX_HANDLE_FIELD: &str = "__h";
/// Hidden field carrying the build fingerprint of the page that rendered a form.
pub const HEMX_BUILD_FIELD: &str = "__b";

/// Request extension that admits `Sec-Fetch-Site: same-site` interactions.
///
/// Add it with `axum::Extension(AllowSameSite)` to a route that sibling
/// subdomains of one site legitimately post to. Cross-site requests stay
/// rejected; this is not an authorization or session boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllowSameSite;
pub const HEMX_RUNTIME_CONTENT_TYPE: &str = "application/javascript; charset=utf-8";
pub const HEMX_SSE_EVENT: &str = "hemx";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeJs;

pub const fn runtime_js() -> RuntimeJs {
    RuntimeJs
}

pub const fn runtime_js_hash() -> &'static str {
    hemx_js::RUNTIME_JS_HASH
}

pub const fn runtime_js_path() -> &'static str {
    hemx_js::RUNTIME_JS_PATH
}

/// The runtime script's URL as a value that Hemplate accepts in `<script src>`.
///
/// Hemplate refuses a plain string there, because even an `https:` URL there
/// would load code. This path is a constant of the build: same-origin and
/// content-addressed. Use it as a template field and no unchecked constructor
/// is needed in application code:
/// `<script src="{+ self.runtime +}" defer></script>`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeScript;

impl std::fmt::Display for RuntimeScript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(runtime_js_path())
    }
}

impl hemplate_runtime::ResourceValue for RuntimeScript {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageMode {
    Full,
    Partial,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageRequest {
    pub mode: PageMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageResponse {
    pub mode: PageMode,
    pub html: String,
    pub title: Option<String>,
    pub fingerprint: Option<BuildFingerprint>,
}

impl PageRequest {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        Self {
            mode: PageMode::from_headers(headers),
        }
    }

    pub const fn is_partial(self) -> bool {
        matches!(self.mode, PageMode::Partial)
    }

    pub fn page(
        self,
        partial_html: impl Into<String>,
        shell: impl FnOnce(String) -> String,
    ) -> PageResponse {
        let partial_html = partial_html.into();
        match self.mode {
            PageMode::Full => PageResponse::full(shell(partial_html)),
            PageMode::Partial => PageResponse::partial(partial_html),
        }
    }

    pub fn page_html<P, S>(self, partial_html: P, shell: impl FnOnce(P) -> S) -> PageResponse
    where
        P: Into<SafeHtml>,
        S: Into<SafeHtml>,
    {
        match self.mode {
            PageMode::Full => PageResponse::full(shell(partial_html).into().into_string()),
            PageMode::Partial => PageResponse::partial(partial_html.into().into_string()),
        }
    }
}

impl<S> FromRequestParts<S> for PageRequest
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self::from_headers(&parts.headers))
    }
}

impl PageResponse {
    pub fn full(html: impl Into<String>) -> Self {
        Self {
            mode: PageMode::Full,
            html: html.into(),
            title: None,
            fingerprint: None,
        }
    }

    pub fn partial(html: impl Into<String>) -> Self {
        Self {
            mode: PageMode::Partial,
            html: html.into(),
            title: None,
            fingerprint: None,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn fingerprint(mut self, fingerprint: BuildFingerprint) -> Self {
        self.fingerprint = Some(fingerprint);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectResponse {
    pub batch: EffectBatch,
}

pub trait DispatchRegistry {
    fn dispatch_form(self, form: InteractionForm) -> Result<EffectResponse, DispatchRejection>;
}

pub trait FromHandlerState<S>: Sized {
    fn from_handler_state(state: S) -> Self;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormDecodeError {
    message: String,
}

type SyncHandler =
    Box<dyn Fn(InteractionForm) -> Result<EffectBatch, DispatchRejection> + Send + Sync>;
type HandlerFuture = Pin<Box<dyn Future<Output = Result<EffectBatch, DispatchRejection>> + Send>>;
type AsyncHandler = Box<dyn Fn(InteractionForm) -> HandlerFuture + Send + Sync>;

pub struct HandlerRegistry {
    fingerprint: BuildFingerprint,
    handlers: BTreeMap<u32, SyncHandler>,
    async_handlers: BTreeMap<u32, AsyncHandler>,
}

pub type Registry = HandlerRegistry;
pub type InteractionHandlers = HandlerRegistry;

pub struct StateHandlerRegistry<S> {
    registry: HandlerRegistry,
    state: S,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InteractionFormRejection {
    CrossSite,
    UnsupportedMediaType,
    BodyTooLarge,
    InvalidBody,
    MissingHandle,
    InvalidHandle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HandlerErrorContext {
    pub handle_id: u32,
    pub fingerprint: BuildFingerprint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HandlerFailure {
    Response { status: StatusCode, message: String },
    Effects(EffectBatch),
}

pub trait IntoHandlerFailure {
    fn into_handler_failure(self, context: HandlerErrorContext) -> HandlerFailure;
}

fn map_handler_failure<E>(
    error: E,
    handle_id: u32,
    fingerprint: BuildFingerprint,
) -> Result<EffectBatch, DispatchRejection>
where
    E: IntoHandlerFailure,
{
    match error.into_handler_failure(HandlerErrorContext {
        handle_id,
        fingerprint,
    }) {
        HandlerFailure::Effects(batch) => Ok(batch),
        failure => Err(DispatchRejection::HandlerError(failure)),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DispatchRejection {
    /// The submitted build fingerprint is missing or differs from `expected`:
    /// the page that rendered the form is older or newer than this server.
    StaleBuild {
        expected: BuildFingerprint,
    },
    UnknownHandle(u32),
    InvalidForm {
        handle_id: u32,
        message: String,
    },
    HandlerError(HandlerFailure),
}

impl HandlerFailure {
    pub fn internal(message: impl Into<String>) -> Self {
        Self::response(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    pub fn response(status: StatusCode, message: impl Into<String>) -> Self {
        Self::Response {
            status,
            message: message.into(),
        }
    }

    pub fn effects(effects: impl IntoEffect, context: HandlerErrorContext) -> Self {
        Self::Effects(effects.into_batch(context.fingerprint))
    }
}

impl EffectResponse {
    pub fn new(effects: impl IntoEffect, fingerprint: BuildFingerprint) -> Self {
        Self {
            batch: effects.into_batch(fingerprint),
        }
    }
}

impl<S> FromHandlerState<S> for S {
    fn from_handler_state(state: S) -> Self {
        state
    }
}

impl<S> FromHandlerState<S> for State<S> {
    fn from_handler_state(state: S) -> Self {
        State(state)
    }
}

pub const fn handlers(fingerprint: BuildFingerprint) -> HandlerRegistry {
    HandlerRegistry::new(fingerprint)
}

pub const fn interactions(fingerprint: BuildFingerprint) -> HandlerRegistry {
    HandlerRegistry::new(fingerprint)
}

pub fn state_interactions<S>(fingerprint: BuildFingerprint, state: S) -> StateHandlerRegistry<S>
where
    S: Clone + Send + Sync + 'static,
{
    interactions(fingerprint).with_state(state)
}

impl InteractionRequest {
    pub fn dispatch(
        self,
        registry: impl DispatchRegistry,
    ) -> Result<EffectResponse, DispatchRejection> {
        registry.dispatch_form(self.form)
    }

    pub async fn dispatch_async(
        self,
        registry: HandlerRegistry,
    ) -> Result<EffectResponse, DispatchRejection> {
        registry.dispatch_async(self.form).await
    }

    pub fn form(&self) -> &InteractionForm {
        &self.form
    }
}

impl From<InteractionForm> for InteractionRequest {
    fn from(form: InteractionForm) -> Self {
        Self { form }
    }
}

impl HandlerRegistry {
    pub const fn new(fingerprint: BuildFingerprint) -> Self {
        Self {
            fingerprint,
            handlers: BTreeMap::new(),
            async_handlers: BTreeMap::new(),
        }
    }

    pub fn register<E>(
        mut self,
        handle_id: u32,
        handler: impl Fn(InteractionForm) -> E + Send + Sync + 'static,
    ) -> Self
    where
        E: IntoEffect,
    {
        let fingerprint = self.fingerprint;
        self.handlers.insert(
            handle_id,
            Box::new(move |form| Ok(handler(form).into_batch(fingerprint))),
        );
        self
    }

    pub fn register_typed<T, E>(
        mut self,
        handle_id: u32,
        handler: impl Fn(T) -> E + Send + Sync + 'static,
    ) -> Self
    where
        T: FromInteractionForm,
        E: IntoEffect,
    {
        let fingerprint = self.fingerprint;
        self.handlers.insert(
            handle_id,
            Box::new(move |form| {
                let handle_id = form.handle_id;
                let input = T::from_interaction_form(&form).map_err(|error| {
                    DispatchRejection::InvalidForm {
                        handle_id,
                        message: error.message,
                    }
                })?;
                Ok(handler(input).into_batch(fingerprint))
            }),
        );
        self
    }

    pub fn register_state<S, C, E>(
        mut self,
        handle_id: u32,
        state: S,
        handler: impl Fn(C) -> E + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        E: IntoEffect,
    {
        let fingerprint = self.fingerprint;
        self.handlers.insert(
            handle_id,
            Box::new(move |_| {
                Ok(handler(C::from_handler_state(state.clone())).into_batch(fingerprint))
            }),
        );
        self
    }

    pub fn register_state_typed<S, C, T, E>(
        mut self,
        handle_id: u32,
        state: S,
        handler: impl Fn(C, T) -> E + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        T: FromInteractionForm,
        E: IntoEffect,
    {
        let fingerprint = self.fingerprint;
        self.handlers.insert(
            handle_id,
            Box::new(move |form| {
                let handle_id = form.handle_id;
                let input = T::from_interaction_form(&form).map_err(|error| {
                    DispatchRejection::InvalidForm {
                        handle_id,
                        message: error.message,
                    }
                })?;
                Ok(handler(C::from_handler_state(state.clone()), input).into_batch(fingerprint))
            }),
        );
        self
    }

    #[doc(hidden)]
    pub fn register_state_result<S, C, E, O>(
        mut self,
        handle_id: u32,
        state: S,
        handler: impl Fn(C) -> Result<O, E> + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        O: IntoEffect,
        E: IntoHandlerFailure,
    {
        let fingerprint = self.fingerprint;
        self.handlers.insert(
            handle_id,
            Box::new(move |form| {
                let handle_id = form.handle_id;
                match handler(C::from_handler_state(state.clone())) {
                    Ok(effects) => Ok(effects.into_batch(fingerprint)),
                    Err(error) => map_handler_failure(error, handle_id, fingerprint),
                }
            }),
        );
        self
    }

    #[doc(hidden)]
    pub fn register_state_typed_result<S, C, T, E, O>(
        mut self,
        handle_id: u32,
        state: S,
        handler: impl Fn(C, T) -> Result<O, E> + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        T: FromInteractionForm,
        O: IntoEffect,
        E: IntoHandlerFailure,
    {
        let fingerprint = self.fingerprint;
        self.handlers.insert(
            handle_id,
            Box::new(move |form| {
                let handle_id = form.handle_id;
                let input = T::from_interaction_form(&form).map_err(|error| {
                    DispatchRejection::InvalidForm {
                        handle_id,
                        message: error.message,
                    }
                })?;
                match handler(C::from_handler_state(state.clone()), input) {
                    Ok(effects) => Ok(effects.into_batch(fingerprint)),
                    Err(error) => map_handler_failure(error, handle_id, fingerprint),
                }
            }),
        );
        self
    }

    pub fn register_async<E, F>(
        mut self,
        handle_id: u32,
        handler: impl Fn(InteractionForm) -> F + Send + Sync + 'static,
    ) -> Self
    where
        F: Future<Output = E> + Send + 'static,
        E: IntoEffect,
    {
        let fingerprint = self.fingerprint;
        self.async_handlers.insert(
            handle_id,
            Box::new(move |form| {
                let future = handler(form);
                Box::pin(async move { Ok(future.await.into_batch(fingerprint)) })
            }),
        );
        self
    }

    pub fn register_typed_async<T, E, F>(
        mut self,
        handle_id: u32,
        handler: impl Fn(T) -> F + Send + Sync + 'static,
    ) -> Self
    where
        T: FromInteractionForm,
        F: Future<Output = E> + Send + 'static,
        E: IntoEffect,
    {
        let fingerprint = self.fingerprint;
        self.async_handlers.insert(
            handle_id,
            Box::new(move |form| {
                let handle_id = form.handle_id;
                let input = match T::from_interaction_form(&form) {
                    Ok(input) => input,
                    Err(error) => {
                        return Box::pin(async move {
                            Err(DispatchRejection::InvalidForm {
                                handle_id,
                                message: error.message,
                            })
                        });
                    }
                };
                let future = handler(input);
                Box::pin(async move { Ok(future.await.into_batch(fingerprint)) })
            }),
        );
        self
    }

    pub fn register_state_async<S, C, E, F>(
        mut self,
        handle_id: u32,
        state: S,
        handler: impl Fn(C) -> F + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        F: Future<Output = E> + Send + 'static,
        E: IntoEffect,
    {
        let fingerprint = self.fingerprint;
        self.async_handlers.insert(
            handle_id,
            Box::new(move |_| {
                let future = handler(C::from_handler_state(state.clone()));
                Box::pin(async move { Ok(future.await.into_batch(fingerprint)) })
            }),
        );
        self
    }

    #[doc(hidden)]
    pub fn register_state_async_result<S, C, E, O, F>(
        mut self,
        handle_id: u32,
        state: S,
        handler: impl Fn(C) -> F + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        F: Future<Output = Result<O, E>> + Send + 'static,
        O: IntoEffect,
        E: IntoHandlerFailure,
    {
        let fingerprint = self.fingerprint;
        self.async_handlers.insert(
            handle_id,
            Box::new(move |form| {
                let handle_id = form.handle_id;
                let future = handler(C::from_handler_state(state.clone()));
                Box::pin(async move {
                    match future.await {
                        Ok(effects) => Ok(effects.into_batch(fingerprint)),
                        Err(error) => map_handler_failure(error, handle_id, fingerprint),
                    }
                })
            }),
        );
        self
    }

    pub fn register_state_typed_async<S, C, T, E, F>(
        mut self,
        handle_id: u32,
        state: S,
        handler: impl Fn(C, T) -> F + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        T: FromInteractionForm,
        F: Future<Output = E> + Send + 'static,
        E: IntoEffect,
    {
        let fingerprint = self.fingerprint;
        self.async_handlers.insert(
            handle_id,
            Box::new(move |form| {
                let handle_id = form.handle_id;
                let input = match T::from_interaction_form(&form) {
                    Ok(input) => input,
                    Err(error) => {
                        return Box::pin(async move {
                            Err(DispatchRejection::InvalidForm {
                                handle_id,
                                message: error.message,
                            })
                        });
                    }
                };
                let future = handler(C::from_handler_state(state.clone()), input);
                Box::pin(async move { Ok(future.await.into_batch(fingerprint)) })
            }),
        );
        self
    }

    #[doc(hidden)]
    pub fn register_state_typed_async_result<S, C, T, E, O, F>(
        mut self,
        handle_id: u32,
        state: S,
        handler: impl Fn(C, T) -> F + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        T: FromInteractionForm,
        F: Future<Output = Result<O, E>> + Send + 'static,
        O: IntoEffect,
        E: IntoHandlerFailure,
    {
        let fingerprint = self.fingerprint;
        self.async_handlers.insert(
            handle_id,
            Box::new(move |form| {
                let handle_id = form.handle_id;
                let input = match T::from_interaction_form(&form) {
                    Ok(input) => input,
                    Err(error) => {
                        return Box::pin(async move {
                            Err(DispatchRejection::InvalidForm {
                                handle_id,
                                message: error.message,
                            })
                        });
                    }
                };
                let future = handler(C::from_handler_state(state.clone()), input);
                Box::pin(async move {
                    match future.await {
                        Ok(effects) => Ok(effects.into_batch(fingerprint)),
                        Err(error) => map_handler_failure(error, handle_id, fingerprint),
                    }
                })
            }),
        );
        self
    }

    pub fn register_handle<I, E>(
        self,
        handle: Handle<I>,
        handler: impl Fn(InteractionForm) -> E + Send + Sync + 'static,
    ) -> Self
    where
        E: IntoEffect,
    {
        self.register(handle.id().id, handler)
    }

    pub fn with_state<S>(self, state: S) -> StateHandlerRegistry<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        StateHandlerRegistry {
            registry: self,
            state,
        }
    }

    pub fn on<I, E>(
        self,
        handle: Handle<I>,
        handler: impl Fn(InteractionForm) -> E + Send + Sync + 'static,
    ) -> Self
    where
        E: IntoEffect,
    {
        self.register_handle(handle, handler)
    }

    pub fn on_form<I, T, E>(
        self,
        handle: Handle<I>,
        handler: impl Fn(T) -> E + Send + Sync + 'static,
    ) -> Self
    where
        T: FromInteractionForm,
        E: IntoEffect,
    {
        self.register_typed(handle.id().id, handler)
    }

    pub fn on_state<I, S, C, E>(
        self,
        handle: Handle<I>,
        state: S,
        handler: impl Fn(C) -> E + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        E: IntoEffect,
    {
        self.register_state(handle.id().id, state, handler)
    }

    pub fn on_state_form<I, S, C, T, E>(
        self,
        handle: Handle<I>,
        state: S,
        handler: impl Fn(C, T) -> E + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        T: FromInteractionForm,
        E: IntoEffect,
    {
        self.register_state_typed(handle.id().id, state, handler)
    }

    #[doc(hidden)]
    pub fn on_state_result<I, S, C, E, O>(
        self,
        handle: Handle<I>,
        state: S,
        handler: impl Fn(C) -> Result<O, E> + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        O: IntoEffect,
        E: IntoHandlerFailure,
    {
        self.register_state_result(handle.id().id, state, handler)
    }

    #[doc(hidden)]
    pub fn on_state_form_result<I, S, C, T, E, O>(
        self,
        handle: Handle<I>,
        state: S,
        handler: impl Fn(C, T) -> Result<O, E> + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        T: FromInteractionForm,
        O: IntoEffect,
        E: IntoHandlerFailure,
    {
        self.register_state_typed_result(handle.id().id, state, handler)
    }

    pub fn on_async<I, E, F>(
        self,
        handle: Handle<I>,
        handler: impl Fn(InteractionForm) -> F + Send + Sync + 'static,
    ) -> Self
    where
        F: Future<Output = E> + Send + 'static,
        E: IntoEffect,
    {
        self.register_async(handle.id().id, handler)
    }

    pub fn on_form_async<I, T, E, F>(
        self,
        handle: Handle<I>,
        handler: impl Fn(T) -> F + Send + Sync + 'static,
    ) -> Self
    where
        T: FromInteractionForm,
        F: Future<Output = E> + Send + 'static,
        E: IntoEffect,
    {
        self.register_typed_async(handle.id().id, handler)
    }

    pub fn on_state_async<I, S, C, E, F>(
        self,
        handle: Handle<I>,
        state: S,
        handler: impl Fn(C) -> F + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        F: Future<Output = E> + Send + 'static,
        E: IntoEffect,
    {
        self.register_state_async(handle.id().id, state, handler)
    }

    #[doc(hidden)]
    pub fn on_state_async_result<I, S, C, E, O, F>(
        self,
        handle: Handle<I>,
        state: S,
        handler: impl Fn(C) -> F + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        F: Future<Output = Result<O, E>> + Send + 'static,
        O: IntoEffect,
        E: IntoHandlerFailure,
    {
        self.register_state_async_result(handle.id().id, state, handler)
    }

    pub fn on_state_form_async<I, S, C, T, E, F>(
        self,
        handle: Handle<I>,
        state: S,
        handler: impl Fn(C, T) -> F + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        T: FromInteractionForm,
        F: Future<Output = E> + Send + 'static,
        E: IntoEffect,
    {
        self.register_state_typed_async(handle.id().id, state, handler)
    }

    #[doc(hidden)]
    pub fn on_state_form_async_result<I, S, C, T, E, O, F>(
        self,
        handle: Handle<I>,
        state: S,
        handler: impl Fn(C, T) -> F + Send + Sync + 'static,
    ) -> Self
    where
        S: Clone + Send + Sync + 'static,
        C: FromHandlerState<S>,
        T: FromInteractionForm,
        F: Future<Output = Result<O, E>> + Send + 'static,
        O: IntoEffect,
        E: IntoHandlerFailure,
    {
        self.register_state_typed_async_result(handle.id().id, state, handler)
    }

    pub fn dispatch(&self, form: InteractionForm) -> Result<EffectResponse, DispatchRejection> {
        form.require_build(self.fingerprint)?;
        let handle_id = form.handle_id;
        let Some(handler) = self.handlers.get(&handle_id) else {
            return Err(DispatchRejection::UnknownHandle(handle_id));
        };
        Ok(EffectResponse {
            batch: handler(form)?,
        })
    }

    pub async fn dispatch_async(
        &self,
        form: InteractionForm,
    ) -> Result<EffectResponse, DispatchRejection> {
        form.require_build(self.fingerprint)?;
        let handle_id = form.handle_id;
        if let Some(handler) = self.async_handlers.get(&handle_id) {
            return Ok(EffectResponse {
                batch: handler(form).await?,
            });
        }
        self.dispatch(form)
    }

    pub fn contains(&self, handle_id: u32) -> bool {
        self.handlers.contains_key(&handle_id) || self.async_handlers.contains_key(&handle_id)
    }
}

impl<S> StateHandlerRegistry<S>
where
    S: Clone + Send + Sync + 'static,
{
    pub fn on_state<I, C, E>(
        mut self,
        handle: Handle<I>,
        handler: impl Fn(C) -> E + Send + Sync + 'static,
    ) -> Self
    where
        C: FromHandlerState<S>,
        E: IntoEffect,
    {
        self.registry = self.registry.on_state(handle, self.state.clone(), handler);
        self
    }

    pub fn on<I, C, T, E>(
        mut self,
        handle: Handle<I>,
        handler: impl Fn(C, T) -> E + Send + Sync + 'static,
    ) -> Self
    where
        C: FromHandlerState<S>,
        T: FromInteractionForm,
        E: IntoEffect,
    {
        self.registry = self
            .registry
            .on_state_form(handle, self.state.clone(), handler);
        self
    }

    #[doc(hidden)]
    pub fn on_state_result<I, C, E, O>(
        mut self,
        handle: Handle<I>,
        handler: impl Fn(C) -> Result<O, E> + Send + Sync + 'static,
    ) -> Self
    where
        C: FromHandlerState<S>,
        O: IntoEffect,
        E: IntoHandlerFailure,
    {
        self.registry = self
            .registry
            .on_state_result(handle, self.state.clone(), handler);
        self
    }

    #[doc(hidden)]
    pub fn on_result<I, C, T, E, O>(
        mut self,
        handle: Handle<I>,
        handler: impl Fn(C, T) -> Result<O, E> + Send + Sync + 'static,
    ) -> Self
    where
        C: FromHandlerState<S>,
        T: FromInteractionForm,
        O: IntoEffect,
        E: IntoHandlerFailure,
    {
        self.registry = self
            .registry
            .on_state_form_result(handle, self.state.clone(), handler);
        self
    }

    pub fn on_state_async<I, C, E, F>(
        mut self,
        handle: Handle<I>,
        handler: impl Fn(C) -> F + Send + Sync + 'static,
    ) -> Self
    where
        C: FromHandlerState<S>,
        F: Future<Output = E> + Send + 'static,
        E: IntoEffect,
    {
        self.registry = self
            .registry
            .on_state_async(handle, self.state.clone(), handler);
        self
    }

    #[doc(hidden)]
    pub fn on_state_async_result<I, C, E, O, F>(
        mut self,
        handle: Handle<I>,
        handler: impl Fn(C) -> F + Send + Sync + 'static,
    ) -> Self
    where
        C: FromHandlerState<S>,
        F: Future<Output = Result<O, E>> + Send + 'static,
        O: IntoEffect,
        E: IntoHandlerFailure,
    {
        self.registry = self
            .registry
            .on_state_async_result(handle, self.state.clone(), handler);
        self
    }

    pub fn on_async<I, C, T, E, F>(
        mut self,
        handle: Handle<I>,
        handler: impl Fn(C, T) -> F + Send + Sync + 'static,
    ) -> Self
    where
        C: FromHandlerState<S>,
        T: FromInteractionForm,
        F: Future<Output = E> + Send + 'static,
        E: IntoEffect,
    {
        self.registry = self
            .registry
            .on_state_form_async(handle, self.state.clone(), handler);
        self
    }

    #[doc(hidden)]
    pub fn on_async_result<I, C, T, E, O, F>(
        mut self,
        handle: Handle<I>,
        handler: impl Fn(C, T) -> F + Send + Sync + 'static,
    ) -> Self
    where
        C: FromHandlerState<S>,
        T: FromInteractionForm,
        F: Future<Output = Result<O, E>> + Send + 'static,
        O: IntoEffect,
        E: IntoHandlerFailure,
    {
        self.registry =
            self.registry
                .on_state_form_async_result(handle, self.state.clone(), handler);
        self
    }

    pub fn into_registry(self) -> HandlerRegistry {
        self.registry
    }
}

impl<S> DispatchRegistry for StateHandlerRegistry<S>
where
    S: Clone + Send + Sync + 'static,
{
    fn dispatch_form(self, form: InteractionForm) -> Result<EffectResponse, DispatchRejection> {
        self.registry.dispatch_form(form)
    }
}

impl DispatchRegistry for HandlerRegistry {
    fn dispatch_form(self, form: InteractionForm) -> Result<EffectResponse, DispatchRejection> {
        self.dispatch(form)
    }
}

impl PageMode {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        match headers
            .get(HEMX_PARTIAL_HEADER)
            .and_then(|value| value.to_str().ok())
        {
            Some("1" | "true") => Self::Partial,
            _ => Self::Full,
        }
    }
}

impl IntoResponse for PageResponse {
    fn into_response(self) -> axum::response::Response {
        let html = match self.fingerprint {
            Some(fingerprint) => html_with_root_fingerprint(self.html, fingerprint),
            None => self.html,
        };
        let mut response = Response::new(Body::from(html));
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        if self.mode == PageMode::Partial {
            response
                .headers_mut()
                .insert(HEMX_PARTIAL_HEADER, HeaderValue::from_static("true"));
        }
        if let Some(fingerprint) = self.fingerprint {
            response
                .headers_mut()
                .insert(HEMX_FINGERPRINT_HEADER, fingerprint_header(fingerprint));
        }
        if let Some(title) = self
            .title
            .and_then(|title| HeaderValue::from_str(&title).ok())
        {
            response.headers_mut().insert(HEMX_TITLE_HEADER, title);
        }
        response
    }
}

/// Stamp every actual `data-hemx-root` start tag that lacks a build fingerprint.
///
/// Scan tags rather than substrings: text, comments, raw text, and attribute
/// values must neither select a different element nor suppress a real root.
fn html_with_root_fingerprint(mut html: String, fingerprint: BuildFingerprint) -> String {
    let lower = html.to_ascii_lowercase(); // ASCII lowering preserves byte offsets.
    let mut stamps = Vec::new();
    let mut at = 0;
    while let Some(offset) = lower[at..].find('<') {
        let start = at + offset;
        if lower[start..].starts_with("<!--") {
            at = lower[start..]
                .find("-->")
                .map_or(lower.len(), |end| start + end + 3);
            continue;
        }
        let Some(tag) = start_tag(&lower, start) else {
            at = start + 1;
            continue;
        };
        if tag.root && !tag.build {
            stamps.push(tag.attributes_end);
        }
        at = tag.end;
        let name = &lower[start + 1..tag.name_end];
        if matches!(
            name,
            "script"
                | "style"
                | "textarea"
                | "title"
                | "xmp"
                | "iframe"
                | "noembed"
                | "noframes"
                | "noscript"
        ) {
            // Raw text and RCDATA cannot contain tags; resume at their end tag.
            let close = format!("</{name}");
            at = lower[at..].find(&close).map_or(lower.len(), |end| at + end);
        }
    }
    let stamp = format!(" data-hemx-build=\"{}\"", fingerprint.0);
    for position in stamps.into_iter().rev() {
        html.insert_str(position, &stamp);
    }
    html
}

struct StartTag {
    name_end: usize,
    attributes_end: usize,
    end: usize,
    root: bool,
    build: bool,
}

/// Parse one start tag in ASCII-lowercased HTML; `None` means "not a start tag".
fn start_tag(html: &str, start: usize) -> Option<StartTag> {
    let bytes = html.as_bytes();
    if !bytes.get(start + 1)?.is_ascii_alphabetic() {
        return None;
    }
    let delimiter = |byte: u8| byte.is_ascii_whitespace() || byte == b'/' || byte == b'>';
    let mut at = start + 1;
    while !delimiter(*bytes.get(at)?) {
        at += 1;
    }
    let mut tag = StartTag {
        name_end: at,
        attributes_end: at,
        end: at,
        root: false,
        build: false,
    };
    loop {
        while bytes.get(at)?.is_ascii_whitespace() || bytes[at] == b'/' {
            at += 1;
        }
        if bytes[at] == b'>' {
            tag.end = at + 1;
            return Some(tag);
        }
        let name_start = at;
        while !delimiter(*bytes.get(at)?) && bytes[at] != b'=' {
            at += 1;
        }
        let name = &html[name_start..at];
        tag.root |= name == "data-hemx-root";
        tag.build |= name == "data-hemx-build";
        let mut value_at = at;
        while bytes.get(value_at)?.is_ascii_whitespace() {
            value_at += 1;
        }
        if bytes[value_at] == b'=' {
            value_at += 1;
            while bytes.get(value_at)?.is_ascii_whitespace() {
                value_at += 1;
            }
            at = match bytes[value_at] {
                quote @ (b'"' | b'\'') => {
                    value_at + 2 + html[value_at + 1..].find(quote as char)?
                }
                _ => {
                    let mut end = value_at;
                    while !bytes.get(end)?.is_ascii_whitespace() && bytes[end] != b'>' {
                        end += 1;
                    }
                    end
                }
            };
        }
        tag.attributes_end = at;
    }
}

fn fingerprint_header(fingerprint: BuildFingerprint) -> HeaderValue {
    HeaderValue::from(fingerprint.0)
}

impl IntoResponse for EffectResponse {
    fn into_response(self) -> axum::response::Response {
        let bytes = match self.batch.to_wire() {
            Ok(bytes) => bytes,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("failed to encode hemx effect response: {error}"),
                )
                    .into_response();
            }
        };
        let mut response = Response::new(Body::from(bytes));
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(HEMX_CONTENT_TYPE),
        );
        response.headers_mut().insert(
            HEMX_FINGERPRINT_HEADER,
            fingerprint_header(self.batch.fingerprint),
        );
        response
    }
}

impl IntoResponse for DispatchRejection {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::StaleBuild { expected } => {
                // The current fingerprint lets the browser recognise a stale page.
                let mut response = (
                    StatusCode::CONFLICT,
                    "this page is out of date; reload it and try again",
                )
                    .into_response();
                response
                    .headers_mut()
                    .insert(HEMX_FINGERPRINT_HEADER, fingerprint_header(expected));
                response
            }
            Self::UnknownHandle(handle_id) => (
                StatusCode::NOT_FOUND,
                format!("unknown hemx handle id {handle_id}"),
            )
                .into_response(),
            Self::InvalidForm { handle_id, message } => (
                StatusCode::BAD_REQUEST,
                format!("invalid hemx form for handle id {handle_id}: {message}"),
            )
                .into_response(),
            Self::HandlerError(HandlerFailure::Response { status, message }) => {
                (status, message).into_response()
            }
            Self::HandlerError(HandlerFailure::Effects(batch)) => {
                EffectResponse { batch }.into_response()
            }
        }
    }
}

impl IntoResponse for RuntimeJs {
    fn into_response(self) -> axum::response::Response {
        let mut response = Response::new(Body::from(hemx_js::RUNTIME_JS));
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(HEMX_RUNTIME_CONTENT_TYPE),
        );
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
        response.headers_mut().insert(
            header::ETAG,
            HeaderValue::from_str(&format!("\"{}\"", runtime_js_hash()))
                .expect("runtime hash is a valid ETag"),
        );
        response.headers_mut().insert(
            header::CONTENT_LENGTH,
            HeaderValue::from(hemx_js::RUNTIME_JS.len() as u64),
        );
        response
    }
}

pub fn runtime_js_source() -> &'static str {
    hemx_js::RUNTIME_JS
}

pub fn runtime_js_script_src() -> &'static str {
    runtime_js_path()
}

pub fn runtime_js_route_path() -> &'static str {
    runtime_js_path()
}

/// Stream canonical batches. Upstream and encoding failures fail the body stream;
/// they are not converted into application-specific error events.
///
/// Items may be owned batches or shared ones such as `Arc<EffectBatch>`, so one
/// broadcast batch fans out to many connections without a copy per client.
pub fn sse<S, B, E>(batches: S) -> Sse<impl Stream<Item = Result<Event, axum::BoxError>> + Send>
where
    S: Stream<Item = Result<B, E>> + Send + 'static,
    B: Borrow<EffectBatch> + Send + 'static,
    E: Into<axum::BoxError>,
{
    Sse::new(Folding::new(batches).map(|frame| {
        let data = encode_sse_batch(frame?.borrow())?;
        Ok(Event::default().event(HEMX_SSE_EVENT).data(data))
    }))
    .keep_alive(KeepAlive::default())
}

/// A batch as received, or the fold of a run of them together with the run's
/// last item, which identifies the state the fold reaches.
enum Frame<B> {
    Received(B),
    Folded(EffectBatch, B),
}

impl<B: Borrow<EffectBatch>> Borrow<EffectBatch> for Frame<B> {
    fn borrow(&self) -> &EffectBatch {
        match self {
            Frame::Received(batch) => batch.borrow(),
            Frame::Folded(batch, _) => batch,
        }
    }
}

/// Batches waiting for the client are coalesced within the wire budget
/// (). Every intermediate operation remains in order; only envelopes
/// are combined. Side effects and differing builds remain separate frames.
struct Folding<S, B> {
    inner: Pin<Box<S>>,
    pending: VecDeque<Frame<B>>,
    error: Option<axum::BoxError>,
    done: bool,
}

impl<S, B> Folding<S, B> {
    fn new(inner: S) -> Self {
        Self {
            inner: Box::pin(inner),
            pending: VecDeque::new(),
            error: None,
            done: false,
        }
    }
}

/// Replace every run of foldable frames by its fold; a run whose fold would
/// exceed the wire budget stays as it is.
fn fold_frames<B: Borrow<EffectBatch>>(frames: Vec<Frame<B>>) -> VecDeque<Frame<B>> {
    fn flush<B: Borrow<EffectBatch>>(run: &mut Vec<Frame<B>>, out: &mut VecDeque<Frame<B>>) {
        let folded = if run.len() > 1 {
            hemx_core::fold_run(run)
        } else {
            None
        };
        match folded {
            Some(folded) => {
                let (Some(Frame::Received(last)) | Some(Frame::Folded(_, last))) = run.pop() else {
                    unreachable!("a folded run has items")
                };
                out.push_back(Frame::Folded(folded, last));
                run.clear();
            }
            None => out.extend(run.drain(..)),
        }
    }
    let mut out = VecDeque::with_capacity(frames.len());
    let mut run: Vec<Frame<B>> = Vec::new();
    for frame in frames {
        let joins = run
            .last()
            .is_some_and(|last| hemx_core::folds_into(last.borrow(), frame.borrow()));
        if !joins {
            flush(&mut run, &mut out);
        }
        run.push(frame);
    }
    flush(&mut run, &mut out);
    out
}

// The inner stream is boxed and pending frames are never pinned, so moving the
// adapter is fine whatever B is.
impl<S, B> Unpin for Folding<S, B> {}

impl<S, B, E> Stream for Folding<S, B>
where
    S: Stream<Item = Result<B, E>>,
    B: Borrow<EffectBatch>,
    E: Into<axum::BoxError>,
{
    type Item = Result<Frame<B>, axum::BoxError>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use std::task::Poll;
        let this = self.get_mut();
        // Take only what is already waiting; never wait for more. The bound keeps
        // an always-ready producer from starving the response.
        let mut ready = Vec::new();
        while !this.done && this.error.is_none() && ready.len() < hemx_core::MAX_EFFECTS {
            match this.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(batch))) => ready.push(Frame::Received(batch)),
                Poll::Ready(Some(Err(error))) => this.error = Some(error.into()),
                Poll::Ready(None) => this.done = true,
                Poll::Pending => break,
            }
        }
        if !ready.is_empty() {
            let waiting: Vec<Frame<B>> = this.pending.drain(..).chain(ready).collect();
            this.pending = fold_frames(waiting);
        }
        if let Some(frame) = this.pending.pop_front() {
            return Poll::Ready(Some(Ok(frame)));
        }
        if let Some(error) = this.error.take() {
            this.done = true;
            return Poll::Ready(Some(Err(error)));
        }
        if this.done {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    }
}

/// Build one SSE event only if the batch is valid and within wire budgets.
pub fn sse_event(batch: EffectBatch) -> Result<Event, WireError> {
    Ok(Event::default()
        .event(HEMX_SSE_EVENT)
        .data(encode_sse_batch(&batch)?))
}

/// Encode canonical wire bytes as unpadded base64url for SSE transport.
pub fn encode_sse_batch(batch: &EffectBatch) -> Result<String, WireError> {
    Ok(base64_url_no_pad(&batch.to_wire()?))
}

fn base64_url_no_pad(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((input.len() * 4).div_ceil(3));
    let (chunks, remainder) = input.as_chunks::<3>();
    for [a, b, c] in chunks {
        out.push(ALPHABET[(a >> 2) as usize] as char);
        out.push(ALPHABET[(((a & 0x03) << 4) + (b >> 4)) as usize] as char);
        out.push(ALPHABET[(((b & 0x0f) << 2) + (c >> 6)) as usize] as char);
        out.push(ALPHABET[(c & 0x3f) as usize] as char);
    }
    match remainder {
        [a] => {
            out.push(ALPHABET[(a >> 2) as usize] as char);
            out.push(ALPHABET[((a & 0x03) << 4) as usize] as char);
        }
        [a, b] => {
            out.push(ALPHABET[(a >> 2) as usize] as char);
            out.push(ALPHABET[(((a & 0x03) << 4) + (b >> 4)) as usize] as char);
            out.push(ALPHABET[((b & 0x0f) << 2) as usize] as char);
        }
        [] => {}
        _ => unreachable!(),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        base64_url_no_pad, encode_sse_batch,
        forms::{parse_urlencoded_pairs, percent_decode},
        html_with_root_fingerprint, sse, BuildFingerprint, InteractionForm,
        InteractionFormRejection, HEMX_SSE_EVENT,
    };
    use axum::{
        body::{to_bytes, Body},
        extract::FromRequest,
        http::{header, Request},
        response::IntoResponse,
    };
    use futures_util::stream;
    use hemx_core::{EffectBatch, EFFECT_BATCH_ABI_VERSION};
    use scraper::{Html, Selector};
    use std::convert::Infallible;

    fn selector(value: &str) -> Selector {
        Selector::parse(value).expect("test selector parses")
    }

    #[test]
    fn root_fingerprint_is_added_to_initial_root() {
        let html = html_with_root_fingerprint(
            "<html><body><main data-hemx-root>Docs</main></body></html>".into(),
            BuildFingerprint(99),
        );

        let document = Html::parse_document(&html);
        let root = document
            .select(&selector("main[data-hemx-root]"))
            .next()
            .expect("root element is rendered");
        assert_eq!(root.value().attr("data-hemx-build"), Some("99"));
        assert_eq!(root.text().collect::<String>(), "Docs");
    }

    #[test]
    fn existing_root_fingerprint_is_preserved() {
        let html = html_with_root_fingerprint(
            "<main data-hemx-root data-hemx-build=\"1\">Docs</main>".into(),
            BuildFingerprint(99),
        );

        let document = Html::parse_fragment(&html);
        let root = document
            .select(&selector("main[data-hemx-root]"))
            .next()
            .expect("root element is rendered");
        assert_eq!(root.value().attr("data-hemx-build"), Some("1"));
        assert_eq!(root.text().collect::<String>(), "Docs");
        assert_eq!(html.matches("data-hemx-build=").count(), 1);
        assert!(!html.contains("data-hemx-build=\"99\""));
    }

    #[test]
    fn every_actual_root_tag_is_stamped_and_text_cannot_redirect_it() {
        let html = html_with_root_fingerprint(
            concat!(
                "<!doctype html><title>data-hemx-root data-hemx-build=</title>",
                "<!-- <aside data-hemx-root> --><script>'<p data-hemx-root>'</script>",
                "<p title=\"a > data-hemx-root\">data-hemx-root</p>",
                "<main id=outer DATA-HEMX-ROOT><section id=nested data-hemx-root='n'/>",
                "<div id=kept data-hemx-root data-hemx-build=\"1\"></div></main>",
            )
            .into(),
            BuildFingerprint(99),
        );
        let document = Html::parse_document(&html);
        let stamped: Vec<_> = document
            .select(&selector("[data-hemx-build]"))
            .map(|element| {
                (
                    element.value().attr("id").unwrap_or(element.value().name()),
                    element.value().attr("data-hemx-build").unwrap(),
                )
            })
            .collect();
        assert_eq!(stamped, [("outer", "99"), ("nested", "99"), ("kept", "1")]);
        let paragraph = document.select(&selector("p")).next().unwrap();
        assert_eq!(paragraph.value().attr("title"), Some("a > data-hemx-root"));
        assert!(html.contains("<title>data-hemx-root data-hemx-build=</title>"));
        assert!(html.contains("<!-- <aside data-hemx-root> -->"));
        assert!(html.contains("<script>'<p data-hemx-root>'</script>"));
    }

    #[test]
    fn fingerprint_injection_leaves_html_without_a_hemx_root_unchanged() {
        let html = "<main>Docs</main>".to_owned();
        assert_eq!(
            html_with_root_fingerprint(html.clone(), BuildFingerprint(99)),
            html
        );
    }

    #[test]
    fn base64url_transport_matches_rfc_4648_vectors_without_padding() {
        for (input, expected) in [
            (b"".as_slice(), ""),
            (b"f".as_slice(), "Zg"),
            (b"fo".as_slice(), "Zm8"),
            (b"foo".as_slice(), "Zm9v"),
            (b"foob".as_slice(), "Zm9vYg"),
            (b"fooba".as_slice(), "Zm9vYmE"),
            (b"foobar".as_slice(), "Zm9vYmFy"),
            (&[0xfb, 0xff, 0xff], "-___"),
            (&[0x00, 0x0f, 0x00], "AA8A"),
            (&[0x00, 0xcf, 0x00], "AM8A"),
            (&[0xff], "_w"),
            (&[0xff, 0xff], "__8"),
        ] {
            assert_eq!(base64_url_no_pad(input), expected);
        }
    }

    #[test]
    fn urlencoded_decoder_handles_standard_escapes_and_rejects_malformed_input() {
        assert_eq!(
            parse_urlencoded_pairs(
                b"empty=&space=+&slash=%2f&digit=%39&upper=%4A&lower=%4a&repeat=1&repeat=2"
            ),
            Ok(vec![
                ("empty".into(), String::new()),
                ("space".into(), " ".into()),
                ("slash".into(), "/".into()),
                ("digit".into(), "9".into()),
                ("upper".into(), "J".into()),
                ("lower".into(), "J".into()),
                ("repeat".into(), "1".into()),
                ("repeat".into(), "2".into()),
            ])
        );
        assert_eq!(parse_urlencoded_pairs(b""), Ok(Vec::new()));
        for malformed in [
            b"bad=%".as_slice(),
            b"bad=%0".as_slice(),
            b"bad=%gg".as_slice(),
            b"bad=%0g".as_slice(),
            b"%gg=value".as_slice(),
        ] {
            assert_eq!(
                parse_urlencoded_pairs(malformed),
                Err(InteractionFormRejection::InvalidBody)
            );
        }
        assert_eq!(percent_decode(b"a+b%2Fc"), Ok("a b/c".into()));
        assert_eq!(
            InteractionForm::parse_urlencoded(b"__h=1&bad=%"),
            Err(InteractionFormRejection::InvalidBody)
        );
    }

    #[tokio::test]
    async fn sse_response_streams_base64_url_effect_batches() {
        let batch = EffectBatch {
            abi_version: EFFECT_BATCH_ABI_VERSION,
            fingerprint: BuildFingerprint(11),
            ops: Vec::new(),
        };
        let encoded = encode_sse_batch(&batch).unwrap();
        assert_eq!(encoded, "SEVNWAMAAAALAAAAAAAAAAAAAAA");

        let response = sse(stream::iter([Ok::<_, Infallible>(batch)])).into_response();
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/event-stream"
        );
        let body = to_bytes(response.into_body(), 1024).await.unwrap();

        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            format!("event: {HEMX_SSE_EVENT}\ndata: {encoded}\n\n")
        );
        assert!(!encoded.contains('='));
    }

    #[tokio::test]
    async fn interaction_form_extracts_multipart_fields_and_files() {
        let boundary = "hemx-test-boundary";
        let body = concat!(
            "--hemx-test-boundary\r\n",
            "Content-Disposition: form-data; name=\"__h\"\r\n\r\n",
            "7\r\n",
            "--hemx-test-boundary\r\n",
            "Content-Disposition: form-data; name=\"title\"\r\n\r\n",
            "Report\r\n",
            "--hemx-test-boundary\r\n",
            "Content-Disposition: form-data; name=\"upload\"; filename=\"a.txt\"\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "hello\r\n",
            "--hemx-test-boundary--\r\n",
        );
        let request = Request::builder()
            .header(
                axum::http::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();

        let form = InteractionForm::from_request(request, &()).await.unwrap();

        assert_eq!(form.handle_id, 7);
        assert_eq!(form.value("title"), Some("Report"));
        let file = form.file("upload").unwrap();
        assert_eq!(file.file_name.as_deref(), Some("a.txt"));
        assert_eq!(file.content_type.as_deref(), Some("text/plain"));
        assert_eq!(file.bytes, b"hello");
    }
}
