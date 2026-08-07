extern crate self as hemx_sync;

use hemx_sync_macros::presence;

pub trait IntoEffect {}

impl IntoEffect for &'static str {}

pub trait PresenceScope {
    fn presence_channel(&self) -> String;
}

pub trait PresenceUpdate {
    fn channel(&self) -> &str;
    fn effect(&self) -> &str;
}

pub struct PresenceProjection<Effect> {
    channel: String,
    effect: Effect,
}

impl<Effect> PresenceProjection<Effect> {
    pub fn new(channel: String, effect: Effect) -> Self {
        Self { channel, effect }
    }
}

impl PresenceUpdate for PresenceProjection<&'static str> {
    fn channel(&self) -> &str {
        &self.channel
    }

    fn effect(&self) -> &str {
        self.effect
    }
}

struct Signal(&'static str);

impl PresenceScope for Signal {
    fn presence_channel(&self) -> String {
        self.0.to_owned()
    }
}

#[presence]
fn project(signal: Signal) -> impl IntoEffect {
    if signal.0 == "board" {
        "joined"
    } else {
        "left"
    }
}

#[test]
fn public_presence_attribute_preserves_the_item_and_expands_it() {
    let update = project(Signal("board"));
    assert_eq!(update.channel(), "board");
    assert_eq!(update.effect(), "joined");
}
