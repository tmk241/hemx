use super::*;

impl EffectBatch {
    /// The versioned hemx codec is the sole public `EffectBatch` wire API.
    /// ```compile_fail
    /// use hemx_core::IntoEffect;
    /// let batch = ().into_batch(hemx_core::BuildFingerprint(1));
    /// let _ = batch.to_postcard();
    /// ```
    /// Return the exact wire size, without validating the ABI or budgets.
    ///
    /// [`Self::to_wire`] validates before allocating or emitting output.
    pub fn encoded_len(&self) -> usize {
        batch_wire_len(self)
    }

    /// Encode a compatible batch within the public wire budgets.
    ///
    /// Validation covers every nested field before the output buffer is allocated.
    pub fn to_wire(&self) -> Result<Vec<u8>, WireError> {
        let encoded_len = validate_batch(self)?;
        let mut out = Vec::with_capacity(encoded_len);
        let initial_capacity = out.capacity();
        write_batch(self, &mut out);
        debug_assert_eq!(out.len(), encoded_len);
        debug_assert_eq!(out.capacity(), initial_capacity);
        Ok(out)
    }

    /// Decode canonical ABI bytes, validating all bounds and tags before allocation.
    pub fn from_wire(bytes: &[u8]) -> Result<Self, WireError> {
        validate_wire(bytes)?;
        read_batch(bytes)
    }

    pub const fn is_compatible(&self) -> bool {
        self.abi_version == EFFECT_BATCH_ABI_VERSION
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireError {
    BadMagic,
    UnsupportedVersion,
    Truncated,
    InvalidUtf8,
    UnknownTag,
    TrailingBytes,
    /// The encoded batch exceeds [`MAX_BATCH_BYTES`].
    BatchTooLarge,
    /// A string or byte field exceeds [`MAX_FIELD_BYTES`].
    FieldTooLarge,
    /// The effect vector exceeds [`MAX_EFFECTS`].
    TooManyEffects,
    /// A Rows declaration orders, or a RowsDelta steps or supplies, more than
    /// [`MAX_ROW_KEYS`] keys.
    TooManyRowKeys,
    /// A row declaration repeats a key, or a Rows declaration supplies a row
    /// outside its order.
    InvalidRows,
}

impl core::fmt::Display for WireError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::BadMagic => "invalid hemx wire magic",
            Self::UnsupportedVersion => "unsupported hemx effect-batch ABI version",
            Self::Truncated => "truncated hemx effect batch",
            Self::InvalidUtf8 => "invalid UTF-8 in hemx wire field",
            Self::UnknownTag => "unknown hemx wire tag",
            Self::TrailingBytes => "trailing bytes after hemx effect batch",
            Self::BatchTooLarge => "hemx effect batch exceeds 1048576 bytes",
            Self::FieldTooLarge => "hemx wire field exceeds 262144 bytes",
            Self::TooManyEffects => "hemx effect batch exceeds 1024 effects",
            Self::TooManyRowKeys => "hemx row declaration exceeds 65536 keys or steps",
            Self::InvalidRows => {
                "hemx row declaration repeats a key or supplies a row outside its order"
            }
        })
    }
}

impl core::error::Error for WireError {}

/// Maximum canonical batch size, including the envelope.
pub const MAX_BATCH_BYTES: usize = 1_048_576;
/// Maximum UTF-8 string or byte payload size, excluding its length prefix.
pub const MAX_FIELD_BYTES: usize = 262_144;
/// Maximum number of effects in one batch.
pub const MAX_EFFECTS: usize = 1_024;
/// Maximum number of keys one Rows declaration may order.
pub const MAX_ROW_KEYS: usize = 65_536;

const WIRE_MAGIC: &[u8; 4] = b"HEMX";
const WIRE_BATCH_HEADER_LEN: usize = WIRE_MAGIC.len() + 4 + 8 + 4;

fn validate_field(len: usize) -> Result<(), WireError> {
    if len > MAX_FIELD_BYTES {
        Err(WireError::FieldTooLarge)
    } else {
        Ok(())
    }
}

fn validate_ref(reference: &ResourceRef) -> Result<(), WireError> {
    match &reference.scope {
        None => Ok(()),
        Some(ScopeKey::KeyValue(value) | ScopeKey::Field(value)) => validate_field(value.len()),
    }
}

/// Structural Rows rules shared by encoding and decoding ().
fn validate_rows(order: &[String], rows: &[(String, SafeHtml)]) -> Result<(), WireError> {
    if order.len() > MAX_ROW_KEYS {
        return Err(WireError::TooManyRowKeys);
    }
    // Sorted slices rather than tree sets: this runs for every encoded batch,
    // and an order can hold tens of thousands of keys.
    let mut ordered: Vec<&str> = order.iter().map(String::as_str).collect();
    ordered.sort_unstable();
    let mut supplied: Vec<&str> = rows.iter().map(|(key, _)| key.as_str()).collect();
    supplied.sort_unstable();
    let duplicate = |keys: &[&str]| keys.windows(2).any(|pair| pair[0] == pair[1]);
    if duplicate(&ordered)
        || duplicate(&supplied)
        || supplied
            .iter()
            .any(|key| ordered.binary_search(key).is_err())
    {
        return Err(WireError::InvalidRows);
    }
    Ok(())
}

/// Structural RowsDelta rules shared by encoding and decoding ().
/// Whether steps apply depends on the client's rows and is checked there.
fn validate_delta(steps: &[RowStep], rows: &[(String, SafeHtml)]) -> Result<(), WireError> {
    if steps.len() > MAX_ROW_KEYS || rows.len() > MAX_ROW_KEYS {
        return Err(WireError::TooManyRowKeys);
    }
    let mut supplied: Vec<&str> = rows.iter().map(|(key, _)| key.as_str()).collect();
    supplied.sort_unstable();
    if supplied.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(WireError::InvalidRows);
    }
    Ok(())
}

fn step_wire_len(step: &RowStep) -> usize {
    1 + match step {
        RowStep::Remove(key) => str_wire_len(key),
        RowStep::Place { key, before } => {
            str_wire_len(key) + 1 + before.as_deref().map_or(0, str_wire_len)
        }
    }
}

pub(crate) fn validate_batch(batch: &EffectBatch) -> Result<usize, WireError> {
    if !batch.is_compatible() {
        return Err(WireError::UnsupportedVersion);
    }
    if batch.ops.len() > MAX_EFFECTS {
        return Err(WireError::TooManyEffects);
    }
    let mut len = WIRE_BATCH_HEADER_LEN;
    for effect in &batch.ops {
        match effect {
            Effect::Patch { target, html, .. } => {
                validate_ref(target)?;
                validate_field(html.as_str().len())?;
            }
            Effect::Rows {
                target,
                order,
                rows,
            } => {
                validate_ref(target)?;
                validate_rows(order, rows)?;
                // Bounded per item, so the running total cannot overflow even on
                // 32-bit hosts before the batch limit stops it.
                len += ref_wire_len(target) + 8;
                for key in order {
                    validate_field(key.len())?;
                    len += str_wire_len(key);
                    if len > MAX_BATCH_BYTES {
                        return Err(WireError::BatchTooLarge);
                    }
                }
                for (key, html) in rows {
                    validate_field(key.len())?;
                    validate_field(html.as_str().len())?;
                    len += str_wire_len(key) + str_wire_len(html.as_str());
                    if len > MAX_BATCH_BYTES {
                        return Err(WireError::BatchTooLarge);
                    }
                }
                len += 1;
                if len > MAX_BATCH_BYTES {
                    return Err(WireError::BatchTooLarge);
                }
                continue;
            }
            Effect::RowsDelta {
                target,
                steps,
                rows,
                ..
            } => {
                validate_ref(target)?;
                validate_delta(steps, rows)?;
                len += 1 + ref_wire_len(target) + 8 + 8 + 4 + 4;
                for step in steps {
                    let (RowStep::Remove(key) | RowStep::Place { key, .. }) = step;
                    validate_field(key.len())?;
                    if let RowStep::Place {
                        before: Some(anchor),
                        ..
                    } = step
                    {
                        validate_field(anchor.len())?;
                    }
                    len += step_wire_len(step);
                    if len > MAX_BATCH_BYTES {
                        return Err(WireError::BatchTooLarge);
                    }
                }
                for (key, html) in rows {
                    validate_field(key.len())?;
                    validate_field(html.as_str().len())?;
                    len += str_wire_len(key) + str_wire_len(html.as_str());
                    if len > MAX_BATCH_BYTES {
                        return Err(WireError::BatchTooLarge);
                    }
                }
                if len > MAX_BATCH_BYTES {
                    return Err(WireError::BatchTooLarge);
                }
                continue;
            }
            Effect::Focus { target, .. } | Effect::Scroll { target, .. } => {
                validate_ref(target)?;
            }
            Effect::Visit { url, .. } => validate_field(url.len())?,
            Effect::Dispatch { event, payload } => {
                validate_field(event.len())?;
                validate_field(payload.len())?;
            }
        }
        // Each effect's bounded fields make this addition safe, including on 32-bit hosts.
        len += effect_wire_len(effect);
        if len > MAX_BATCH_BYTES {
            return Err(WireError::BatchTooLarge);
        }
    }
    Ok(len)
}

fn batch_wire_len(batch: &EffectBatch) -> usize {
    WIRE_BATCH_HEADER_LEN + batch.ops.iter().map(effect_wire_len).sum::<usize>()
}

fn effect_wire_len(effect: &Effect) -> usize {
    1 + match effect {
        Effect::Patch { target, html, .. } => {
            ref_wire_len(target) + 1 + str_wire_len(html.as_str())
        }
        Effect::Rows {
            target,
            order,
            rows,
        } => {
            ref_wire_len(target)
                + 8
                + order.iter().map(|key| str_wire_len(key)).sum::<usize>()
                + rows
                    .iter()
                    .map(|(key, html)| str_wire_len(key) + str_wire_len(html.as_str()))
                    .sum::<usize>()
        }
        Effect::RowsDelta {
            target,
            steps,
            rows,
            ..
        } => {
            ref_wire_len(target)
                + 8
                + 8
                + 4
                + steps.iter().map(step_wire_len).sum::<usize>()
                + 4
                + rows
                    .iter()
                    .map(|(key, html)| str_wire_len(key) + str_wire_len(html.as_str()))
                    .sum::<usize>()
        }
        Effect::Focus { target, .. } => ref_wire_len(target) + 1,
        Effect::Scroll { target, .. } => ref_wire_len(target) + 2,
        Effect::Visit { url, .. } => str_wire_len(url) + 1,
        Effect::Dispatch { event, payload } => {
            str_wire_len(event.as_str()) + str_wire_len_bytes(payload)
        }
    }
}

fn str_wire_len_bytes(value: &[u8]) -> usize {
    4 + value.len()
}

fn ref_wire_len(reference: &ResourceRef) -> usize {
    1 + 4
        + 1
        + match &reference.scope {
            None => 0,
            Some(ScopeKey::KeyValue(value) | ScopeKey::Field(value)) => str_wire_len(value),
        }
}

const fn str_wire_len(value: &str) -> usize {
    4 + value.len()
}

fn write_batch(batch: &EffectBatch, out: &mut Vec<u8>) {
    out.extend_from_slice(WIRE_MAGIC);
    write_u32(batch.abi_version, out);
    write_u64(batch.fingerprint.0, out);
    write_u32(batch.ops.len() as u32, out);
    for op in &batch.ops {
        write_effect(op, out);
    }
}

fn write_effect(effect: &Effect, out: &mut Vec<u8>) {
    match effect {
        Effect::Patch { target, mode, html } => {
            write_u8(0, out);
            write_ref(target, out);
            write_u8(
                match mode {
                    PatchMode::Morph => 0,
                    PatchMode::Replace => 1,
                },
                out,
            );
            write_str(html.as_str(), out);
        }
        Effect::Rows {
            target,
            order,
            rows,
        } => {
            write_u8(1, out);
            write_ref(target, out);
            write_u32(order.len() as u32, out);
            for key in order {
                write_str(key, out);
            }
            write_u32(rows.len() as u32, out);
            for (key, html) in rows {
                write_str(key, out);
                write_str(html.as_str(), out);
            }
        }
        Effect::Focus {
            target,
            prevent_scroll,
        } => {
            write_u8(2, out);
            write_ref(target, out);
            write_u8(u8::from(*prevent_scroll), out);
        }
        Effect::Scroll {
            target,
            block,
            behavior,
        } => {
            write_u8(3, out);
            write_ref(target, out);
            write_u8(
                match block {
                    ScrollBlock::Start => 0,
                    ScrollBlock::Center => 1,
                    ScrollBlock::End => 2,
                    ScrollBlock::Nearest => 3,
                },
                out,
            );
            write_u8(
                match behavior {
                    ScrollBehavior::Auto => 0,
                    ScrollBehavior::Instant => 1,
                    ScrollBehavior::Smooth => 2,
                },
                out,
            );
        }
        Effect::Visit { url, history } => {
            write_u8(4, out);
            write_str(url, out);
            write_u8(
                match history {
                    HistoryMode::Push => 0,
                    HistoryMode::Replace => 1,
                },
                out,
            );
        }
        Effect::Dispatch { event, payload } => {
            write_u8(5, out);
            write_str(event.as_str(), out);
            write_bytes(payload, out);
        }
        Effect::RowsDelta {
            target,
            from,
            to,
            steps,
            rows,
        } => {
            write_u8(6, out);
            write_ref(target, out);
            write_u64(*from, out);
            write_u64(*to, out);
            write_u32(steps.len() as u32, out);
            for step in steps {
                match step {
                    RowStep::Remove(key) => {
                        write_u8(0, out);
                        write_str(key, out);
                    }
                    RowStep::Place { key, before } => {
                        write_u8(1, out);
                        write_str(key, out);
                        match before {
                            None => write_u8(0, out),
                            Some(anchor) => {
                                write_u8(1, out);
                                write_str(anchor, out);
                            }
                        }
                    }
                }
            }
            write_u32(rows.len() as u32, out);
            for (key, html) in rows {
                write_str(key, out);
                write_str(html.as_str(), out);
            }
        }
    }
}

fn write_ref(reference: &ResourceRef, out: &mut Vec<u8>) {
    write_u8(
        match reference.resource.kind {
            ResourceKind::Slot => 0,
            ResourceKind::Handle => 2,
            ResourceKind::Form => 3,
        },
        out,
    );
    write_u32(reference.resource.id, out);
    match &reference.scope {
        None => write_u8(0, out),
        Some(ScopeKey::KeyValue(value)) => {
            write_u8(1, out);
            write_str(value, out);
        }
        Some(ScopeKey::Field(value)) => {
            write_u8(2, out);
            write_str(value, out);
        }
    }
}

fn write_bytes(value: &[u8], out: &mut Vec<u8>) {
    write_u32(value.len() as u32, out);
    out.extend_from_slice(value);
}

fn write_str(value: &str, out: &mut Vec<u8>) {
    write_u32(value.len() as u32, out);
    out.extend_from_slice(value.as_bytes());
}

fn write_u8(value: u8, out: &mut Vec<u8>) {
    out.push(value);
}

fn write_u32(value: u32, out: &mut Vec<u8>) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_u64(value: u64, out: &mut Vec<u8>) {
    out.extend_from_slice(&value.to_le_bytes());
}

struct WireReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> WireReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn finish(&self) -> Result<(), WireError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(WireError::TrailingBytes)
        }
    }

    fn read_u8(&mut self) -> Result<u8, WireError> {
        let bytes = self.read_exact(1)?;
        Ok(bytes[0])
    }

    fn read_u32(&mut self) -> Result<u32, WireError> {
        let bytes = self.read_exact(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_u64(&mut self) -> Result<u64, WireError> {
        let bytes = self.read_exact(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn read_str(&mut self) -> Result<&'a str, WireError> {
        core::str::from_utf8(self.read_bytes()?).map_err(|_| WireError::InvalidUtf8)
    }

    fn read_bytes(&mut self) -> Result<&'a [u8], WireError> {
        let len = self.read_u32()? as usize;
        if len > MAX_FIELD_BYTES {
            return Err(WireError::FieldTooLarge);
        }
        self.read_exact(len)
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8], WireError> {
        let remaining = &self.bytes[self.offset..];
        let bytes = remaining.get(..len).ok_or(WireError::Truncated)?;
        self.offset += len;
        Ok(bytes)
    }
}

fn read_header(reader: &mut WireReader<'_>) -> Result<(u32, BuildFingerprint, u32), WireError> {
    if reader.bytes.len() > MAX_BATCH_BYTES {
        return Err(WireError::BatchTooLarge);
    }
    if reader.read_exact(WIRE_MAGIC.len())? != WIRE_MAGIC {
        return Err(WireError::BadMagic);
    }
    let abi_version = reader.read_u32()?;
    if abi_version != EFFECT_BATCH_ABI_VERSION {
        return Err(WireError::UnsupportedVersion);
    }
    let fingerprint = BuildFingerprint(reader.read_u64()?);
    let ops_len = reader.read_u32()?;
    if ops_len as usize > MAX_EFFECTS {
        return Err(WireError::TooManyEffects);
    }
    Ok((abi_version, fingerprint, ops_len))
}

fn read_batch(bytes: &[u8]) -> Result<EffectBatch, WireError> {
    let mut reader = WireReader::new(bytes);
    let (abi_version, fingerprint, ops_len) = read_header(&mut reader)?;
    let mut ops = Vec::with_capacity(ops_len as usize);
    for _ in 0..ops_len {
        ops.push(read_effect(&mut reader)?);
    }
    reader.finish()?;
    Ok(EffectBatch {
        abi_version,
        fingerprint,
        ops,
    })
}

fn read_effect(reader: &mut WireReader<'_>) -> Result<Effect, WireError> {
    match reader.read_u8()? {
        0 => Ok(Effect::Patch {
            target: read_ref(reader)?,
            mode: match reader.read_u8()? {
                0 => PatchMode::Morph,
                1 => PatchMode::Replace,
                _ => return Err(WireError::UnknownTag),
            },
            html: SafeHtml::trusted(reader.read_str()?),
        }),
        1 => read_rows(reader),
        2 => Ok(Effect::Focus {
            target: read_ref(reader)?,
            prevent_scroll: match reader.read_u8()? {
                0 => false,
                1 => true,
                _ => return Err(WireError::UnknownTag),
            },
        }),
        3 => Ok(Effect::Scroll {
            target: read_ref(reader)?,
            block: match reader.read_u8()? {
                0 => ScrollBlock::Start,
                1 => ScrollBlock::Center,
                2 => ScrollBlock::End,
                3 => ScrollBlock::Nearest,
                _ => return Err(WireError::UnknownTag),
            },
            behavior: match reader.read_u8()? {
                0 => ScrollBehavior::Auto,
                1 => ScrollBehavior::Instant,
                2 => ScrollBehavior::Smooth,
                _ => return Err(WireError::UnknownTag),
            },
        }),
        4 => Ok(Effect::Visit {
            url: reader.read_str()?.to_string(),
            history: match reader.read_u8()? {
                0 => HistoryMode::Push,
                1 => HistoryMode::Replace,
                _ => return Err(WireError::UnknownTag),
            },
        }),
        5 => Ok(Effect::Dispatch {
            event: reader.read_str()?.to_string(),
            payload: reader.read_bytes()?.to_vec(),
        }),
        6 => read_delta(reader),
        _ => Err(WireError::UnknownTag),
    }
}

fn read_rows(reader: &mut WireReader<'_>) -> Result<Effect, WireError> {
    let target = read_ref(reader)?;
    let order_len = reader.read_u32()? as usize;
    if order_len > MAX_ROW_KEYS {
        return Err(WireError::TooManyRowKeys);
    }
    // Every key costs at least its 4-byte length, so the remaining input bounds
    // the allocation before any key is read.
    let mut order = Vec::with_capacity(order_len.min(reader.remaining() / 4));
    for _ in 0..order_len {
        order.push(reader.read_str()?.to_string());
    }
    let rows_len = reader.read_u32()? as usize;
    if rows_len > order_len {
        return Err(WireError::InvalidRows);
    }
    let mut rows = Vec::with_capacity(rows_len.min(reader.remaining() / 8));
    for _ in 0..rows_len {
        let key = reader.read_str()?.to_string();
        rows.push((key, SafeHtml::trusted(reader.read_str()?)));
    }
    validate_rows(&order, &rows)?;
    Ok(Effect::Rows {
        target,
        order,
        rows,
    })
}

fn read_delta(reader: &mut WireReader<'_>) -> Result<Effect, WireError> {
    let target = read_ref(reader)?;
    let from = reader.read_u64()?;
    let to = reader.read_u64()?;
    let steps_len = reader.read_u32()? as usize;
    if steps_len > MAX_ROW_KEYS {
        return Err(WireError::TooManyRowKeys);
    }
    // Every step costs at least a tag and a 4-byte length.
    let mut steps = Vec::with_capacity(steps_len.min(reader.remaining() / 5));
    for _ in 0..steps_len {
        steps.push(match reader.read_u8()? {
            0 => RowStep::Remove(reader.read_str()?.to_string()),
            1 => RowStep::Place {
                key: reader.read_str()?.to_string(),
                before: match reader.read_u8()? {
                    0 => None,
                    1 => Some(reader.read_str()?.to_string()),
                    _ => return Err(WireError::UnknownTag),
                },
            },
            _ => return Err(WireError::UnknownTag),
        });
    }
    let rows_len = reader.read_u32()? as usize;
    if rows_len > MAX_ROW_KEYS {
        return Err(WireError::TooManyRowKeys);
    }
    let mut rows = Vec::with_capacity(rows_len.min(reader.remaining() / 8));
    for _ in 0..rows_len {
        let key = reader.read_str()?.to_string();
        rows.push((key, SafeHtml::trusted(reader.read_str()?)));
    }
    validate_delta(&steps, &rows)?;
    Ok(Effect::RowsDelta {
        target,
        from,
        to,
        steps,
        rows,
    })
}

fn read_ref(reader: &mut WireReader<'_>) -> Result<ResourceRef, WireError> {
    let kind = match reader.read_u8()? {
        0 => ResourceKind::Slot,
        2 => ResourceKind::Handle,
        3 => ResourceKind::Form,
        _ => return Err(WireError::UnknownTag),
    };
    let resource = ResourceId::new(kind, reader.read_u32()?);
    let scope = match reader.read_u8()? {
        0 => None,
        1 => Some(ScopeKey::KeyValue(reader.read_str()?.to_string())),
        2 => Some(ScopeKey::Field(reader.read_str()?.to_string())),
        _ => return Err(WireError::UnknownTag),
    };
    Ok(ResourceRef { resource, scope })
}

// A borrowing preflight validates the entire input before constructing any owned
// effects. Even a malformed final field cannot trigger an earlier allocation.
fn validate_wire(bytes: &[u8]) -> Result<(), WireError> {
    let mut reader = WireReader::new(bytes);
    let (_, _, count) = read_header(&mut reader)?;
    for _ in 0..count {
        match reader.read_u8()? {
            0 => {
                scan_ref(&mut reader)?;
                scan_tag(&mut reader, 1)?;
                reader.read_str()?;
            }
            1 => {
                scan_ref(&mut reader)?;
                let order = reader.read_u32()? as usize;
                if order > MAX_ROW_KEYS {
                    return Err(WireError::TooManyRowKeys);
                }
                for _ in 0..order {
                    reader.read_str()?;
                }
                let rows = reader.read_u32()? as usize;
                if rows > order {
                    return Err(WireError::InvalidRows);
                }
                for _ in 0..rows {
                    reader.read_str()?;
                    reader.read_str()?;
                }
            }
            2 => {
                scan_ref(&mut reader)?;
                scan_tag(&mut reader, 1)?;
            }
            3 => {
                scan_ref(&mut reader)?;
                scan_tag(&mut reader, 3)?;
                scan_tag(&mut reader, 2)?;
            }
            4 => {
                reader.read_str()?;
                scan_tag(&mut reader, 1)?;
            }
            5 => {
                reader.read_str()?;
                reader.read_bytes()?;
            }
            6 => {
                scan_ref(&mut reader)?;
                reader.read_u64()?;
                reader.read_u64()?;
                let steps = reader.read_u32()? as usize;
                if steps > MAX_ROW_KEYS {
                    return Err(WireError::TooManyRowKeys);
                }
                for _ in 0..steps {
                    let place = scan_tag(&mut reader, 1)? == 1;
                    reader.read_str()?;
                    if place && scan_tag(&mut reader, 1)? == 1 {
                        reader.read_str()?;
                    }
                }
                let rows = reader.read_u32()? as usize;
                if rows > MAX_ROW_KEYS {
                    return Err(WireError::TooManyRowKeys);
                }
                for _ in 0..rows {
                    reader.read_str()?;
                    reader.read_str()?;
                }
            }
            _ => return Err(WireError::UnknownTag),
        }
    }
    reader.finish()
}

fn scan_tag(reader: &mut WireReader<'_>, max: u8) -> Result<u8, WireError> {
    let tag = reader.read_u8()?;
    if tag > max {
        return Err(WireError::UnknownTag);
    }
    Ok(tag)
}

fn scan_ref(reader: &mut WireReader<'_>) -> Result<(), WireError> {
    match reader.read_u8()? {
        0 | 2 | 3 => {}
        _ => return Err(WireError::UnknownTag),
    }
    reader.read_u32()?;
    match scan_tag(reader, 2)? {
        0 => {}
        _ => {
            reader.read_str()?;
        }
    }
    Ok(())
}
