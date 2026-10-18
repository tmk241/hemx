use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractionFile {
    pub name: String,
    pub file_name: Option<String>,
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractionForm {
    pub handle_id: u32,
    fields: Vec<(String, String)>,
    files: Vec<InteractionFile>,
}

/// A validated hemx mutation request.
///
/// Only `application/x-www-form-urlencoded` and `multipart/form-data` are
/// accepted. Body size is intentionally host policy: apply Axum's
/// [`axum::extract::DefaultBodyLimit`] (or a compatible request-body limit)
/// to the mutation route; limit rejections become HTTP 413 before dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractionRequest {
    pub(super) form: InteractionForm,
}

pub trait FromInteractionForm: Sized {
    fn from_interaction_form(form: &InteractionForm) -> Result<Self, FormDecodeError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Form<T>(pub T);

impl<T> Form<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> std::ops::Deref for Form<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> FromInteractionForm for T
where
    T: FromForm,
{
    fn from_interaction_form(form: &InteractionForm) -> Result<Self, FormDecodeError> {
        T::from_form_fields(form.fields())
            .map_err(|error| FormDecodeError::new(error.message().to_owned()))
    }
}

impl<T> FromInteractionForm for Form<T>
where
    T: FromForm,
{
    fn from_interaction_form(form: &InteractionForm) -> Result<Self, FormDecodeError> {
        T::from_form_fields(form.fields())
            .map(Self)
            .map_err(|error| FormDecodeError::new(error.message().to_owned()))
    }
}

impl FormDecodeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl InteractionForm {
    pub fn new(handle_id: u32, fields: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            handle_id,
            fields: fields.into_iter().collect(),
            files: Vec::new(),
        }
    }

    pub fn for_handle<I>(
        handle: Handle<I>,
        fields: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        Self::new(handle.id().id, fields)
    }

    /// The build fingerprint submitted with this form, if present and well formed.
    pub fn build_fingerprint(&self) -> Option<BuildFingerprint> {
        self.value(HEMX_BUILD_FIELD)?
            .parse()
            .ok()
            .map(BuildFingerprint)
    }

    /// Set the submitted build fingerprint, as a generated form would render it.
    pub fn with_build(mut self, fingerprint: BuildFingerprint) -> Self {
        self.fields.retain(|(name, _)| name != HEMX_BUILD_FIELD);
        self.fields
            .push((HEMX_BUILD_FIELD.to_owned(), fingerprint.0.to_string()));
        self
    }

    /// Reject a form rendered by a different build than `current`.
    ///
    /// Registry dispatch calls this before any handler. Applications that read
    /// the form directly must call it themselves; skipping it bypasses the guard.
    pub fn require_build(&self, current: BuildFingerprint) -> Result<(), DispatchRejection> {
        if self.build_fingerprint() == Some(current) {
            Ok(())
        } else {
            Err(DispatchRejection::StaleBuild { expected: current })
        }
    }

    pub fn parse_urlencoded(body: &[u8]) -> Result<Self, InteractionFormRejection> {
        Self::from_parts(parse_urlencoded_pairs(body)?, Vec::new())
    }

    pub async fn parse_multipart(
        mut multipart: Multipart,
    ) -> Result<Self, InteractionFormRejection> {
        let mut fields = Vec::new();
        let mut files = Vec::new();

        while let Some(field) = multipart.next_field().await.map_err(extractor_rejection)? {
            let Some(name) = field.name().map(str::to_owned) else {
                continue;
            };
            let file_name = field.file_name().map(str::to_owned);
            let content_type = field.content_type().map(str::to_owned);
            let bytes = field.bytes().await.map_err(extractor_rejection)?;

            if file_name.is_some() {
                files.push(InteractionFile {
                    name,
                    file_name,
                    content_type,
                    bytes: bytes.to_vec(),
                });
            } else {
                let value = String::from_utf8(bytes.to_vec())
                    .map_err(|_| InteractionFormRejection::InvalidBody)?;
                fields.push((name, value));
            }
        }

        Self::from_parts(fields, files)
    }

    fn from_parts(
        fields: Vec<(String, String)>,
        files: Vec<InteractionFile>,
    ) -> Result<Self, InteractionFormRejection> {
        let Some(handle) = fields
            .iter()
            .find_map(|(name, value)| (name == HEMX_HANDLE_FIELD).then_some(value))
        else {
            return Err(InteractionFormRejection::MissingHandle);
        };
        let handle_id = handle
            .parse::<u32>()
            .map_err(|_| InteractionFormRejection::InvalidHandle)?;
        Ok(Self {
            handle_id,
            fields,
            files,
        })
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find_map(|(field, value)| (field == name).then_some(value.as_str()))
    }

    pub fn parse<T>(&self, name: &str) -> Option<T>
    where
        T: std::str::FromStr,
    {
        self.value(name).and_then(|value| value.parse().ok())
    }

    pub fn values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.fields
            .iter()
            .filter_map(move |(field, value)| (field == name).then_some(value.as_str()))
    }

    pub fn fields(&self) -> &[(String, String)] {
        &self.fields
    }

    pub fn files(&self) -> &[InteractionFile] {
        &self.files
    }

    pub fn file(&self, name: &str) -> Option<&InteractionFile> {
        self.files.iter().find(|file| file.name == name)
    }

    pub fn required(&self, name: &str) -> Result<&str, FormDecodeError> {
        self.value(name)
            .ok_or_else(|| FormDecodeError::new(format!("missing form field `{name}`")))
    }

    pub fn parse_required<T>(&self, name: &str) -> Result<T, FormDecodeError>
    where
        T: std::str::FromStr,
    {
        self.required(name)?
            .parse()
            .map_err(|_| FormDecodeError::new(format!("invalid form field `{name}`")))
    }
}

impl IntoResponse for InteractionFormRejection {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            Self::CrossSite => (
                StatusCode::FORBIDDEN,
                "hemx interactions are not accepted from another site",
            ),
            Self::UnsupportedMediaType => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "hemx interactions require application/x-www-form-urlencoded or multipart/form-data",
            ),
            Self::BodyTooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "hemx interaction body exceeds the host limit"),
            Self::InvalidBody => (StatusCode::BAD_REQUEST, "invalid hemx form body"),
            Self::MissingHandle => (StatusCode::BAD_REQUEST, "missing __h hemx handle field"),
            Self::InvalidHandle => (StatusCode::BAD_REQUEST, "invalid __h hemx handle field"),
        };
        (status, message).into_response()
    }
}

impl<S> FromRequest<S> for InteractionRequest
where
    S: Send + Sync,
{
    type Rejection = InteractionFormRejection;

    async fn from_request(request: Request<Body>, state: &S) -> Result<Self, Self::Rejection> {
        InteractionForm::from_request(request, state)
            .await
            .map(|form| Self { form })
    }
}

impl<S> FromRequest<S> for InteractionForm
where
    S: Send + Sync,
{
    type Rejection = InteractionFormRejection;

    async fn from_request(request: Request<Body>, state: &S) -> Result<Self, Self::Rejection> {
        // Decide on the browser-supplied request context before reading any body.
        check_fetch_site(&request)?;
        match interaction_media_type(request.headers())? {
            InteractionMediaType::Multipart => {
                let multipart = Multipart::from_request(request, state)
                    .await
                    .map_err(extractor_rejection)?;
                Self::parse_multipart(multipart).await
            }
            InteractionMediaType::UrlEncoded => {
                let bytes = Bytes::from_request(request, state)
                    .await
                    .map_err(extractor_rejection)?;
                Self::parse_urlencoded(&bytes)
            }
        }
    }
}

/// Reject requests the browser itself marks as cross-site. A missing header
/// (older browsers, non-browser clients) is not a decision either way.
fn check_fetch_site(request: &Request<Body>) -> Result<(), InteractionFormRejection> {
    let site = request
        .headers()
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok());
    match site {
        Some("cross-site") => Err(InteractionFormRejection::CrossSite),
        Some("same-site") if request.extensions().get::<AllowSameSite>().is_none() => {
            Err(InteractionFormRejection::CrossSite)
        }
        _ => Ok(()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InteractionMediaType {
    Multipart,
    UrlEncoded,
}

fn interaction_media_type(
    headers: &HeaderMap,
) -> Result<InteractionMediaType, InteractionFormRejection> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .ok_or(InteractionFormRejection::UnsupportedMediaType)?;
    match content_type
        .split(';')
        .next()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("multipart/form-data") => Ok(InteractionMediaType::Multipart),
        Some("application/x-www-form-urlencoded") => Ok(InteractionMediaType::UrlEncoded),
        _ => Err(InteractionFormRejection::UnsupportedMediaType),
    }
}

fn extractor_rejection(rejection: impl IntoResponse) -> InteractionFormRejection {
    if rejection.into_response().status() == StatusCode::PAYLOAD_TOO_LARGE {
        InteractionFormRejection::BodyTooLarge
    } else {
        InteractionFormRejection::InvalidBody
    }
}

pub(super) fn parse_urlencoded_pairs(
    body: &[u8],
) -> Result<Vec<(String, String)>, InteractionFormRejection> {
    if body.is_empty() {
        return Ok(Vec::new());
    }

    body.split(|byte| *byte == b'&')
        .map(|pair| {
            let equals = pair.iter().position(|byte| *byte == b'=');
            let (name, value) = match equals {
                Some(index) => (&pair[..index], &pair[index + 1..]),
                None => (pair, &[][..]),
            };
            Ok((percent_decode(name)?, percent_decode(value)?))
        })
        .collect()
}

pub(super) fn percent_decode(input: &[u8]) -> Result<String, InteractionFormRejection> {
    let mut out = Vec::with_capacity(input.len());
    let mut bytes = input.iter().copied();
    while let Some(byte) = bytes.next() {
        match byte {
            b'+' => out.push(b' '),
            b'%' => {
                let high = bytes
                    .next()
                    .and_then(hex)
                    .ok_or(InteractionFormRejection::InvalidBody)?;
                let low = bytes
                    .next()
                    .and_then(hex)
                    .ok_or(InteractionFormRejection::InvalidBody)?;
                out.push(high * 16 + low);
            }
            byte => out.push(byte),
        }
    }
    String::from_utf8(out).map_err(|_| InteractionFormRejection::InvalidBody)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
