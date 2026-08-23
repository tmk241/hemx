use hemx_core::{Atom, BuildFingerprint, Effect, EffectBatch, IntoEffect};
use serde::{de, Deserialize, Deserializer, Serialize};
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
    hash::Hash,
};

extern crate self as hemx_sync;
pub use hemx_sync_macros::presence;

pub const PATCH_SCHEMA_VERSION: u16 = 1;
pub const PATCH_EVENT: &str = "hemx:sync-patch";
pub const ACK_EVENT: &str = "hemx:sync-ack";
const INTERACTION_ID: &str = "$hemx-interaction";
pub const BROWSER_RUNTIME: &str = include_str!("../runtime/hemx-sync.js");

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Channel(String);

impl Channel {
    pub fn new(value: impl Into<String>) -> Result<Self, ChannelError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ChannelError::Empty);
        }
        if value.len() > 128 {
            return Err(ChannelError::TooLong);
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-' | b'.'))
        {
            return Err(ChannelError::InvalidCharacter);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelError {
    Empty,
    TooLong,
    InvalidCharacter,
}

impl fmt::Display for ChannelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("sync channel must not be empty"),
            Self::TooLong => formatter.write_str("sync channel is too long"),
            Self::InvalidCharacter => {
                formatter.write_str("sync channel contains an invalid character")
            }
        }
    }
}

impl Error for ChannelError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresenceChange {
    pub changed: bool,
    pub count: usize,
}

#[derive(Clone, Debug)]
pub struct PresenceTracker<Member> {
    members: HashMap<Channel, HashSet<Member>>,
}

impl<Member> Default for PresenceTracker<Member> {
    fn default() -> Self {
        Self {
            members: HashMap::new(),
        }
    }
}

impl<Member> PresenceTracker<Member>
where
    Member: Eq + Hash,
{
    pub fn join(&mut self, channel: Channel, member: Member) -> PresenceChange {
        let members = self.members.entry(channel).or_default();
        PresenceChange {
            changed: members.insert(member),
            count: members.len(),
        }
    }

    pub fn leave(&mut self, channel: &Channel, member: &Member) -> PresenceChange {
        let Some(members) = self.members.get_mut(channel) else {
            return PresenceChange {
                changed: false,
                count: 0,
            };
        };
        let changed = members.remove(member);
        let count = members.len();
        if members.is_empty() {
            self.members.remove(channel);
        }
        PresenceChange { changed, count }
    }

    pub fn count(&self, channel: &Channel) -> usize {
        self.members.get(channel).map_or(0, HashSet::len)
    }
}

pub trait PresenceScope {
    fn presence_channel(&self) -> Channel;
}

pub struct PresenceProjection<Effect> {
    channel: Channel,
    effect: Effect,
}

impl<Effect> PresenceProjection<Effect> {
    pub fn new(channel: Channel, effect: Effect) -> Self {
        Self { channel, effect }
    }
}

pub trait PresenceUpdate: IntoEffect + Sized {
    fn presence_channel(&self) -> &Channel;
    fn into_broadcast(self, fingerprint: hemx_core::BuildFingerprint) -> Broadcast;
}

impl<Effect> PresenceUpdate for PresenceProjection<Effect>
where
    Effect: IntoEffect,
{
    fn presence_channel(&self) -> &Channel {
        &self.channel
    }

    fn into_broadcast(self, fingerprint: hemx_core::BuildFingerprint) -> Broadcast {
        SyncEffect::broadcast(self.channel, self.effect.into_batch(fingerprint))
    }
}

impl<Effect> IntoEffect for PresenceProjection<Effect>
where
    Effect: IntoEffect,
{
    fn append_to(self, ops: &mut Vec<hemx_core::Effect>) {
        self.effect.append_to(ops);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Broadcast {
    channel: Channel,
    effect_batch: EffectBatch,
}

impl Broadcast {
    pub fn channel(&self) -> &Channel {
        &self.channel
    }

    pub fn effect_batch(&self) -> &EffectBatch {
        &self.effect_batch
    }

    pub fn into_parts(self) -> (Channel, EffectBatch) {
        (self.channel, self.effect_batch)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PatchValue {
    Boolean(bool),
    Integer(i64),
    String(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlatPatch {
    schema_version: u16,
    idempotency_key: String,
    operation_id: String,
    key: String,
    value: PatchValue,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FlatPatchWire {
    schema_version: u16,
    idempotency_key: String,
    operation_id: String,
    key: String,
    value: PatchValue,
}

impl<'de> Deserialize<'de> for FlatPatch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = FlatPatchWire::deserialize(deserializer)?;
        let patch = Self {
            schema_version: wire.schema_version,
            idempotency_key: wire.idempotency_key,
            operation_id: wire.operation_id,
            key: wire.key,
            value: wire.value,
        };
        patch.validate().map_err(de::Error::custom)?;
        Ok(patch)
    }
}

impl FlatPatch {
    pub fn for_interaction(key: impl Into<String>, value: PatchValue) -> Result<Self, PatchError> {
        let key = key.into();
        validate_key(&key)?;
        validate_value(&value)?;
        Ok(Self {
            schema_version: PATCH_SCHEMA_VERSION,
            idempotency_key: INTERACTION_ID.to_owned(),
            operation_id: INTERACTION_ID.to_owned(),
            key,
            value,
        })
    }

    pub fn new(
        idempotency_key: impl Into<String>,
        operation_id: impl Into<String>,
        key: impl Into<String>,
        value: PatchValue,
    ) -> Result<Self, PatchError> {
        let patch = Self {
            schema_version: PATCH_SCHEMA_VERSION,
            idempotency_key: idempotency_key.into(),
            operation_id: operation_id.into(),
            key: key.into(),
            value,
        };
        patch.validate()?;
        Ok(patch)
    }

    pub fn payload(&self) -> String {
        let value = match &self.value {
            PatchValue::Boolean(value) => value.to_string(),
            PatchValue::Integer(value) => value.to_string(),
            PatchValue::String(value) => json_string(value),
        };
        format!(
            r#"{{"schemaVersion":{},"idempotencyKey":{},"operationId":{},"key":{},"value":{value}}}"#,
            self.schema_version,
            json_string(&self.idempotency_key),
            json_string(&self.operation_id),
            json_string(&self.key),
        )
    }

    pub fn validate(&self) -> Result<(), PatchError> {
        if self.schema_version != PATCH_SCHEMA_VERSION {
            return Err(PatchError::SchemaVersion(self.schema_version));
        }
        if self.idempotency_key != INTERACTION_ID || self.operation_id != INTERACTION_ID {
            validate_identifier("idempotency_key", &self.idempotency_key, 128)?;
            validate_identifier("operation_id", &self.operation_id, 128)?;
        }
        validate_key(&self.key)?;
        validate_value(&self.value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatchError {
    Empty(&'static str),
    InvalidCharacter(&'static str),
    TooLong(&'static str),
    ReservedKey,
    SchemaVersion(u16),
    ValueTooLong,
    IntegerOutOfRange,
}

impl fmt::Display for PatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(field) => write!(formatter, "{field} must not be empty"),
            Self::InvalidCharacter(field) => {
                write!(formatter, "{field} contains an invalid character")
            }
            Self::TooLong(field) => write!(formatter, "{field} is too long"),
            Self::ReservedKey => formatter.write_str("patch key is reserved"),
            Self::SchemaVersion(version) => {
                write!(formatter, "unsupported patch schema version {version}")
            }
            Self::ValueTooLong => formatter.write_str("patch string value is too long"),
            Self::IntegerOutOfRange => {
                formatter.write_str("patch integer value exceeds JavaScript's safe range")
            }
        }
    }
}

impl Error for PatchError {}

fn json_string(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() + 2);
    encoded.push('"');
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            character if character <= '\u{1f}' => {
                let byte = character as u8;
                encoded.push_str("\\u00");
                encoded.push(HEX[(byte >> 4) as usize] as char);
                encoded.push(HEX[(byte & 0x0f) as usize] as char);
            }
            character => encoded.push(character),
        }
    }
    encoded.push('"');
    encoded
}

fn validate_value(value: &PatchValue) -> Result<(), PatchError> {
    match value {
        PatchValue::String(value) if value.len() > 4096 => Err(PatchError::ValueTooLong),
        PatchValue::Integer(value) if value.unsigned_abs() > 9_007_199_254_740_991 => {
            Err(PatchError::IntegerOutOfRange)
        }
        _ => Ok(()),
    }
}

fn validate_identifier(field: &'static str, value: &str, maximum: usize) -> Result<(), PatchError> {
    if value.is_empty() {
        return Err(PatchError::Empty(field));
    }
    if value.len() > maximum {
        return Err(PatchError::TooLong(field));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-' | b'.'))
    {
        return Err(PatchError::InvalidCharacter(field));
    }
    Ok(())
}

fn validate_key(key: &str) -> Result<(), PatchError> {
    if key.is_empty() {
        return Err(PatchError::Empty("key"));
    }
    if key.len() > 64 {
        return Err(PatchError::TooLong("key"));
    }
    if matches!(
        key,
        "schemaVersion" | "idempotencyKey" | "operationId" | "key" | "value"
    ) {
        return Err(PatchError::ReservedKey);
    }
    let mut bytes = key.bytes();
    if !bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(PatchError::InvalidCharacter("key"));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncEffect(Vec<Effect>); //

impl SyncEffect {
    pub fn broadcast(channel: Channel, effect_batch: EffectBatch) -> Broadcast {
        Broadcast {
            channel,
            effect_batch,
        }
    }

    pub fn ack<T>(atom: Atom<T>) -> Self {
        Self(vec![
            atom.set("acknowledged"),
            Effect::Emit {
                name: ACK_EVENT.to_owned(),
                payload: format!(r#"{{"atomId":{}}}"#, atom.id().id),
            },
        ])
    }

    pub fn send_patch(patch: FlatPatch) -> Self {
        Self(vec![Effect::Emit {
            name: PATCH_EVENT.to_owned(),
            payload: patch.payload(),
        }])
    }

    /// Apply an optimistic projection now and carry the same ordinary batch in
    /// the durable patch event so the framework sync runtime can replay it
    /// after reload before acknowledgement.
    pub fn durable(
        patch: FlatPatch,
        projection: impl IntoEffect,
        fingerprint: BuildFingerprint,
    ) -> Self {
        let projection = projection.into_batch(fingerprint);
        let projection_wire = projection
            .to_wire()
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let payload = format!(
            r#"{{"patch":{},"projection":[{projection_wire}]}}"#,
            patch.payload()
        );
        let mut ops = projection.ops;
        ops.push(Effect::Emit {
            name: PATCH_EVENT.to_owned(),
            payload,
        });
        Self(ops)
    }
}

impl IntoEffect for SyncEffect {
    fn append_to(self, ops: &mut Vec<Effect>) {
        ops.extend(self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_patch_carries_and_applies_ordinary_projection_batch() {
        let patch =
            FlatPatch::for_interaction("cardColumn", PatchValue::String("done".into())).unwrap();
        let projection = Effect::Emit {
            name: "projected".into(),
            payload: "card:1".into(),
        };
        let batch = SyncEffect::durable(patch, projection, hemx_core::BuildFingerprint(7))
            .into_batch(hemx_core::BuildFingerprint(7));
        assert_eq!(batch.ops.len(), 2);
        assert!(matches!(&batch.ops[0], Effect::Emit { name, .. } if name == "projected"));
        let Effect::Emit { name, payload } = &batch.ops[1] else {
            panic!("durable sync must end with its patch event");
        };
        assert_eq!(name, PATCH_EVENT);
        let payload: serde_json::Value = serde_json::from_str(payload).unwrap();
        let expected_projection = EffectBatch {
            abi_version: hemx_core::EFFECT_BATCH_ABI_VERSION,
            fingerprint: BuildFingerprint(7),
            ops: vec![Effect::Emit {
                name: "projected".into(),
                payload: "card:1".into(),
            }],
        }
        .to_wire();
        assert_eq!(
            payload["projection"],
            serde_json::Value::Array(
                expected_projection
                    .into_iter()
                    .map(serde_json::Value::from)
                    .collect()
            )
        );
        assert_eq!(payload["patch"]["idempotencyKey"], INTERACTION_ID);
    }

    #[test]
    fn acknowledgement_updates_atom_and_emits_queue_signal() {
        let atom = Atom::<String>::new(17);
        let batch = SyncEffect::ack(atom).into_batch(hemx_core::BuildFingerprint(3));
        assert!(
            matches!(&batch.ops[0], Effect::Put { target, payload: hemx_core::Payload::Text(payload) } if target.resource == atom.id() && payload == "acknowledged")
        );
        assert!(
            matches!(&batch.ops[1], Effect::Emit { name, payload } if name == ACK_EVENT && payload == r#"{"atomId":17}"#)
        );
    }

    #[test]
    fn presence_macro_projects_an_ordinary_effect_on_its_channel() {
        struct Signal(Channel);
        impl PresenceScope for Signal {
            fn presence_channel(&self) -> Channel {
                self.0.clone()
            }
        }
        #[presence]
        fn project(signal: Signal) -> impl IntoEffect {
            Effect::Emit {
                name: "presence".to_owned(),
                payload: signal.0.as_str().to_owned(),
            }
        }

        let update = project(Signal(Channel::new("board").unwrap()));
        assert_eq!(update.presence_channel().as_str(), "board");
        let broadcast = update.into_broadcast(hemx_core::BuildFingerprint(9));
        assert_eq!(broadcast.channel().as_str(), "board");
        assert_eq!(broadcast.effect_batch().ops.len(), 1);
    }

    #[test]
    fn presence_tracker_is_idempotent_and_channel_scoped() {
        let alpha = Channel::new("board:alpha").unwrap();
        let beta = Channel::new("board:beta").unwrap();
        let mut tracker = PresenceTracker::default();
        assert_eq!(
            tracker.join(alpha.clone(), "ada"),
            PresenceChange {
                changed: true,
                count: 1
            }
        );
        assert_eq!(
            tracker.join(alpha.clone(), "ada"),
            PresenceChange {
                changed: false,
                count: 1
            }
        );
        assert_eq!(tracker.join(beta.clone(), "ada").count, 1);
        assert_eq!(tracker.leave(&alpha, &"ada").count, 0);
        assert_eq!(tracker.count(&beta), 1);
    }

    #[test]
    fn broadcast_preserves_typed_channel_and_ordinary_batch() {
        let channel = Channel::new("board:alpha").unwrap();
        let batch = EffectBatch {
            abi_version: 1,
            fingerprint: hemx_core::BuildFingerprint(7),
            ops: vec![],
        };
        let broadcast = SyncEffect::broadcast(channel.clone(), batch.clone());
        assert_eq!(broadcast.into_parts(), (channel, batch));
        assert_eq!(
            Channel::new("board alpha"),
            Err(ChannelError::InvalidCharacter)
        );
    }

    #[test]
    fn channel_boundary_and_errors_are_explicit() {
        let valid = format!("a{}", "x".repeat(127));
        assert_eq!(Channel::new(&valid).unwrap().as_str(), valid);
        for (value, expected, message) in [
            (
                "".to_owned(),
                ChannelError::Empty,
                "sync channel must not be empty",
            ),
            (
                format!("a{}", "x".repeat(128)),
                ChannelError::TooLong,
                "sync channel is too long",
            ),
            (
                "board/alpha".to_owned(),
                ChannelError::InvalidCharacter,
                "sync channel contains an invalid character",
            ),
        ] {
            let error = Channel::new(value).expect_err("invalid channel must fail closed");
            assert_eq!(error, expected);
            assert_eq!(error.to_string(), message);
        }
    }

    #[test]
    fn presence_leave_and_projection_preserve_observable_state() {
        let channel = Channel::new("board").unwrap();
        let mut tracker = PresenceTracker::default();
        assert_eq!(
            tracker.leave(&channel, &"missing"),
            PresenceChange {
                changed: false,
                count: 0,
            }
        );
        tracker.join(channel.clone(), "ada");
        tracker.join(channel.clone(), "grace");
        assert_eq!(tracker.count(&channel), 2);
        assert_eq!(
            tracker.leave(&channel, &"ada"),
            PresenceChange {
                changed: true,
                count: 1,
            }
        );
        assert_eq!(tracker.count(&channel), 1);
        assert_eq!(tracker.leave(&channel, &"grace").count, 0);
        assert_eq!(tracker.count(&channel), 0);
        assert!(!tracker.members.contains_key(&channel));

        let effect = Effect::Emit {
            name: "presence".into(),
            payload: "joined".into(),
        };
        let projection = PresenceProjection::new(channel, effect.clone());
        assert_eq!(projection.into_batch(BuildFingerprint(1)).ops, vec![effect]);
    }

    #[test]
    fn flat_patch_enforces_identifier_key_and_value_boundaries() {
        let valid_identifier = format!("a{}", "x".repeat(127));
        let valid_key = format!("a{}", "x".repeat(63));
        let patch = FlatPatch::new(
            &valid_identifier,
            &valid_identifier,
            &valid_key,
            PatchValue::String("x".repeat(4096)),
        )
        .expect("documented patch limits are inclusive");
        assert_eq!(patch.validate(), Ok(()));

        assert_eq!(
            FlatPatch::new("", "operation", "field", PatchValue::Boolean(true)),
            Err(PatchError::Empty("idempotency_key"))
        );
        assert_eq!(
            FlatPatch::new("actor", "", "field", PatchValue::Boolean(true)),
            Err(PatchError::Empty("operation_id"))
        );
        assert_eq!(
            FlatPatch::new(
                "actor",
                "operation",
                format!("a{}", "x".repeat(64)),
                PatchValue::Boolean(true),
            ),
            Err(PatchError::TooLong("key"))
        );
        let cases = [
            FlatPatch::new(INTERACTION_ID, "", "field", PatchValue::Boolean(true)),
            FlatPatch::new("", INTERACTION_ID, "field", PatchValue::Boolean(true)),
            FlatPatch::new("actor", "", "field", PatchValue::Boolean(true)),
            FlatPatch::new(
                format!("a{}", "x".repeat(128)),
                "operation",
                "field",
                PatchValue::Boolean(true),
            ),
            FlatPatch::new("actor", "bad operation", "field", PatchValue::Boolean(true)),
            FlatPatch::new(
                "actor",
                format!("a{}", "x".repeat(128)),
                "field",
                PatchValue::Boolean(true),
            ),
            FlatPatch::new("actor", "operation", "", PatchValue::Boolean(true)),
            FlatPatch::new(
                "actor",
                "operation",
                format!("a{}", "x".repeat(64)),
                PatchValue::Boolean(true),
            ),
            FlatPatch::new("actor", "operation", "1field", PatchValue::Boolean(true)),
            FlatPatch::new("actor", "operation", "field.dot", PatchValue::Boolean(true)),
            FlatPatch::new(
                "actor",
                "operation",
                "field",
                PatchValue::String("x".repeat(4097)),
            ),
            FlatPatch::new(
                "actor",
                "operation",
                "field",
                PatchValue::Integer(9_007_199_254_740_992),
            ),
            FlatPatch::new(
                "actor",
                "operation",
                "field",
                PatchValue::Integer(-9_007_199_254_740_992),
            ),
        ];
        for result in cases {
            assert!(result.is_err(), "invalid patch boundary must fail closed");
        }
        assert!(FlatPatch::new(
            "actor",
            "operation",
            "field",
            PatchValue::Integer(-9_007_199_254_740_991),
        )
        .is_ok());
        assert_eq!(
            FlatPatch::for_interaction("", PatchValue::Boolean(true)),
            Err(PatchError::Empty("key"))
        );
        assert_eq!(
            FlatPatch::for_interaction("field", PatchValue::String("x".repeat(4097)),),
            Err(PatchError::ValueTooLong)
        );
        assert_eq!(PatchError::ReservedKey.to_string(), "patch key is reserved");
        assert_eq!(
            PatchError::ValueTooLong.to_string(),
            "patch string value is too long"
        );
    }

    #[test]
    fn flat_patch_json_round_trips_escaped_values_and_rejects_invalid_input() {
        let escaped = "quote:\" slash:\\ newline:\n return:\r tab:\t control:\u{1f}";
        let patch = FlatPatch::new(
            "actor:1",
            "operation-1",
            "field_name",
            PatchValue::String(escaped.into()),
        )
        .unwrap();
        let payload = patch.payload();
        let decoded: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(decoded["value"], escaped);
        assert_eq!(serde_json::from_str::<FlatPatch>(&payload).unwrap(), patch);

        for (json, expected) in [
            (
                r#"{"schemaVersion":2,"idempotencyKey":"actor","operationId":"op","key":"field","value":true}"#,
                "unsupported patch schema version 2",
            ),
            (
                r#"{"schemaVersion":1,"idempotencyKey":"actor","operationId":"op","key":"field","value":true,"extra":1}"#,
                "unknown field `extra`",
            ),
            (
                r#"{"schemaVersion":1,"idempotencyKey":"actor","operationId":"op","key":"1field","value":true}"#,
                "key contains an invalid character",
            ),
        ] {
            assert!(
                serde_json::from_str::<FlatPatch>(json)
                    .unwrap_err()
                    .to_string()
                    .contains(expected),
                "invalid JSON must report {expected}"
            );
        }
    }

    #[test]
    fn send_patch_emits_the_canonical_payload() {
        let patch = FlatPatch::for_interaction("done", PatchValue::Boolean(true)).unwrap();
        assert_eq!(patch.validate(), Ok(()));
        let expected = patch.payload();
        assert_eq!(
            SyncEffect::send_patch(patch)
                .into_batch(BuildFingerprint(4))
                .ops,
            vec![Effect::Emit {
                name: PATCH_EVENT.into(),
                payload: expected,
            }]
        );
    }

    #[test]
    fn schema_is_flat_and_rejects_reserved_keys() {
        let patch = FlatPatch::new(
            "actor:1",
            "move-card-to-done",
            "cardColumn",
            PatchValue::String("done".to_owned()),
        )
        .unwrap();
        assert_eq!(
            patch.payload(),
            r#"{"schemaVersion":1,"idempotencyKey":"actor:1","operationId":"move-card-to-done","key":"cardColumn","value":"done"}"#
        );
        assert_eq!(
            FlatPatch::new("actor:1", "move", "value", PatchValue::Integer(1)),
            Err(PatchError::ReservedKey)
        );
        assert_eq!(
            FlatPatch::new(
                "actor:1",
                "move",
                "rank",
                PatchValue::Integer(9_007_199_254_740_992),
            ),
            Err(PatchError::IntegerOutOfRange)
        );
        assert!(serde_json::from_str::<FlatPatch>(
            r#"{"schemaVersion":1,"idempotencyKey":"actor:1","operationId":"move","key":"rank","value":9007199254740992}"#,
        )
        .unwrap_err()
        .to_string()
        .contains("safe range"));
    }
}
