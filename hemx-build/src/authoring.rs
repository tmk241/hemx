use super::*;
use crate::artifact::{event_tokens, static_attr, unkeyed_generated_target_diagnostic_for_scope};

pub(super) fn reject_selector_target_attrs(
    path: &Path,
    attrs: &[SurfaceAttribute],
) -> io::Result<()> {
    for attr in attrs {
        let name = attr.name.as_str();
        if matches!(name, "data-hemx-target" | "data-hemx-select") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}: `{name}` is selector-style targeting; hemx uses generated resources instead. Add data-hemx-slot to the local element and return an effect for that generated slot.",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

pub(super) fn reject_unknown_hemx_attrs(path: &Path, attrs: &[SurfaceAttribute]) -> io::Result<()> {
    for attr in attrs {
        let name = attr.name.as_str();
        if name == "data-hemx-ws" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}: `data-hemx-ws` was removed; Hemx streams server updates with `data-hemx-sse`. An application that needs a WebSocket owns it and passes canonical batches to `window.hemx.applyBatch`",
                    path.display()
                ),
            ));
        }
        if name.starts_with("data-hemx-")
            && !known_hemx_attr(name)
            && !name.starts_with(crate::artifact::HANDLE_PARAM_PREFIX)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}: unknown hemx attribute `{name}`; check the spelling or use a non-hemx data-* attribute for app-specific metadata",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

pub(super) fn known_hemx_attr(name: &str) -> bool {
    matches!(
        name,
        "data-hemx-root"
            | "data-hemx-sse"
            | "data-hemx-handle"
            | "data-hemx-slot"
            | "data-hemx-collection"
            | "data-hemx-form"
            | "data-hemx-key"
            | "data-hemx-on"
            | "data-hemx-pending-class"
            | "data-hemx-indicator"
            | "data-hemx-confirm"
            | "data-hemx-debounce"
            | "data-hemx-throttle"
            | "data-hemx-every"
            | "data-hemx-interval"
            | "data-hemx-revealed"
            | "data-hemx-revealed-ahead"
            | "data-hemx-disable-while-pending"
            | "data-hemx-policy"
            | "data-hemx-nav"
            | "data-hemx-history"
            | "data-hemx-boost"
            | "data-hemx-error-for"
            | "data-hemx-error"
            | "data-hemx-island"
    )
}

pub(super) fn reject_invalid_hemx_attr_values(
    path: &Path,
    attrs: &[SurfaceAttribute],
) -> io::Result<()> {
    for attr in attrs
        .iter()
        .filter(|attr| attr.origin == AttributeOrigin::Static)
    {
        let value = attr.value.as_deref().unwrap_or("");
        match attr.name.as_str() {
            "data-hemx-policy" if !valid_policy(value) => {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected one of `latest`, `queue`, `drop`, or `parallel`",
                ));
            }
            "data-hemx-history" if !matches!(value.trim(), "" | "push" | "replace") => {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected `push`, `replace`, or empty for the default push behavior",
                ));
            }
            "data-hemx-on" if !valid_event_list(value) => {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected runtime-supported events: `click`, `submit`, `input`, `change`, `keydown`, `dragstart`, `dragover`, or `drop`",
                ));
            }
            "data-hemx-confirm" if value.trim().is_empty() => {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected a non-empty confirmation message",
                ));
            }
            "data-hemx-sse" if value.trim().is_empty() => {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected a non-empty same-origin SSE URL",
                ));
            }
            "data-hemx-revealed-ahead"
                if !value
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .is_some_and(|value| value.is_finite() && value >= 0.0) =>
            {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected a non-negative number of viewports",
                ));
            }
            "data-hemx-debounce" | "data-hemx-throttle" | "data-hemx-every"
            | "data-hemx-interval"
                if !valid_duration(value) =>
            {
                return Err(invalid_hemx_value(
                    path,
                    &attr.name,
                    value,
                    "expected milliseconds like `250`/`250ms` or seconds like `1s`",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn reject_invalid_hemx_attr_placement(
    path: &Path,
    tag: &str,
    attrs: &[SurfaceAttribute],
) -> io::Result<()> {
    // These declarations all lower to the single data-hemx-resource marker.
    // Handles lower to data-hemx-action, so action + form remains valid.
    for (first, second) in [
        ("data-hemx-slot", "data-hemx-collection"),
        ("data-hemx-slot", "data-hemx-form"),
        ("data-hemx-collection", "data-hemx-form"),
    ] {
        if has_attr(attrs, first) && has_attr(attrs, second) {
            return Err(invalid_hemx_placement(
                path,
                second,
                &format!(
                    "an element cannot declare both `{first}` and `{second}`; both lower to the single `data-hemx-resource` marker; use separate elements"
                ),
            ));
        }
    }

    if has_attr(attrs, "data-hemx-collection")
        && matches!(
            tag,
            "area"
                | "base"
                | "br"
                | "col"
                | "embed"
                | "hr"
                | "img"
                | "input"
                | "link"
                | "meta"
                | "param"
                | "source"
                | "track"
                | "wbr"
        )
    {
        return Err(invalid_hemx_placement(
            path,
            "data-hemx-collection",
            "expected a non-void element that can contain rows",
        ));
    }
    if has_attr(attrs, "data-hemx-nav")
        && (tag != "a"
            || !has_attr(attrs, "href")
            || static_attr(attrs, "href").is_some_and(|href| href.trim().is_empty()))
    {
        return Err(invalid_hemx_placement(
            path,
            "data-hemx-nav",
            "expected a real `<a href=...>` link so navigation works without JavaScript",
        ));
    }
    if has_attr(attrs, "data-hemx-boost") && matches!(tag, "a" | "form") {
        return Err(invalid_hemx_placement(
            path,
            "data-hemx-boost",
            "expected a container around descendant links/forms; use `data-hemx-nav` on anchors or `data-hemx-handle` on forms",
        ));
    }
    if has_attr(attrs, "data-hemx-sse") && !has_attr(attrs, "data-hemx-root") {
        return Err(invalid_hemx_placement(
            path,
            "data-hemx-sse",
            "expected placement on the same element as `data-hemx-root`",
        ));
    }
    Ok(())
}

fn has_attr(attrs: &[SurfaceAttribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.name == name)
}

fn invalid_hemx_placement(path: &Path, attr: &str, expectation: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "{}: invalid {attr} placement; {expectation}",
            path.display()
        ),
    )
}

fn valid_policy(value: &str) -> bool {
    matches!(value.trim(), "latest" | "queue" | "drop" | "parallel")
}

fn valid_event_list(value: &str) -> bool {
    let mut events = event_tokens(value).peekable();
    events.peek().is_some() && events.all(valid_runtime_event)
}

fn valid_runtime_event(value: &str) -> bool {
    matches!(
        value,
        "click" | "submit" | "input" | "change" | "keydown" | "dragstart" | "dragover" | "drop"
    )
}

fn valid_duration(value: &str) -> bool {
    let value = value.trim();
    let digits = value
        .strip_suffix("ms")
        .or_else(|| value.strip_suffix('s'))
        .unwrap_or(value);
    !digits.is_empty() && digits.as_bytes().iter().all(u8::is_ascii_digit)
}

fn invalid_hemx_value(path: &Path, attr: &str, value: &str, expectation: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "{}: invalid {attr} value `{value}`; {expectation}",
            path.display()
        ),
    )
}

pub(super) fn reject_unkeyed_loop(
    surface: &SurfaceDocument,
    scope: ScopeId,
    path: &Path,
    kind: &str,
    name: &str,
) -> io::Result<()> {
    if let Some(diagnostic) =
        unkeyed_generated_target_diagnostic_for_scope(surface, scope, path, kind, name)
    {
        Err(diagnostic.to_io_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_attribute_value_validators_cover_every_boundary() {
        for valid in ["latest", " queue ", "drop", "parallel"] {
            assert!(valid_policy(valid), "rejected policy {valid:?}");
        }
        for invalid in ["", "newest", "latest queue", "LATEST"] {
            assert!(!valid_policy(invalid), "accepted policy {invalid:?}");
        }

        for valid in [
            "click",
            "submit input change keydown dragstart dragover drop",
            " click change ",
        ] {
            assert!(valid_event_list(valid), "rejected event list {valid:?}");
        }
        for invalid in ["", " ", "blur", "click blur"] {
            assert!(
                !valid_event_list(invalid),
                "accepted event list {invalid:?}"
            );
        }
        for valid in [
            "click",
            "submit",
            "input",
            "change",
            "keydown",
            "dragstart",
            "dragover",
            "drop",
        ] {
            assert!(valid_runtime_event(valid), "rejected event {valid:?}");
        }
        for invalid in ["", "blur", "Click"] {
            assert!(!valid_runtime_event(invalid), "accepted event {invalid:?}");
        }

        for valid in ["0", "250", "250ms", "1s", " 5s "] {
            assert!(valid_duration(valid), "rejected duration {valid:?}");
        }
        for invalid in ["", " ", "ms", "s", "-1", "1sec", "1.5s", "1 ms"] {
            assert!(!valid_duration(invalid), "accepted duration {invalid:?}");
        }
    }
}
