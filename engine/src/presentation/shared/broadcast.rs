//! Routed WebSocket broadcast envelope (C-WS).
//!
//! The engine fans out every serialized message on a single
//! `tokio::broadcast` channel. Most messages are *public* market data
//! (price ticks, candles, strategy signals) that every connected client may
//! see, but **order updates carry one user's private trading activity** —
//! their pair, quantity, fill price and strategy. Delivering those to every
//! socket is a cross-user financial-data disclosure on a multi-tenant
//! platform.
//!
//! Rather than embedding routing hints inside the JSON the client receives,
//! we wrap each payload in a [`BroadcastEnvelope`] carrying an out-of-band
//! [`Audience`]. The WS handler knows the authenticated `user_id` of its
//! session and consults [`Audience::should_deliver_to`] before forwarding —
//! so a private envelope never reaches a socket it is not addressed to.

/// Who a broadcast message may be delivered to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Audience {
    /// Public market data (prices, candles, signals) — delivered to every
    /// connected session, authenticated or not.
    Public,
    /// Private to a single user (e.g. their order updates) — delivered only to
    /// sessions authenticated as that exact `user_id`.
    User(i32),
}

impl Audience {
    /// Routing decision: may a session whose authenticated user is
    /// `session_user` (`None` = unauthenticated) receive this audience?
    ///
    /// Public is delivered to everyone. A `User(uid)` envelope is delivered
    /// only when the session is authenticated as that same `uid` — an
    /// unauthenticated session (`None`) never receives private data, and one
    /// user never receives another user's private data.
    pub fn should_deliver_to(&self, session_user: Option<i32>) -> bool {
        match self {
            Audience::Public => true,
            Audience::User(uid) => session_user == Some(*uid),
        }
    }
}

/// A serialized payload tagged with the [`Audience`] allowed to receive it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BroadcastEnvelope {
    pub audience: Audience,
    pub payload: String,
}

impl BroadcastEnvelope {
    /// Public market data — delivered to every session.
    pub fn public(payload: String) -> Self {
        Self {
            audience: Audience::Public,
            payload,
        }
    }

    /// Private to one user. `None` (an order with no owning user — e.g. a
    /// system/manual placement) falls back to [`Audience::Public`]: there is
    /// no individual to scope it to, and these never carry another user's
    /// data. Subscription-driven orders always carry the subscriber's id and
    /// are therefore scoped to that user.
    pub fn for_user(user_id: Option<i32>, payload: String) -> Self {
        match user_id {
            Some(uid) => Self {
                audience: Audience::User(uid),
                payload,
            },
            None => Self::public(payload),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_is_delivered_to_unauthenticated_session() {
        assert!(Audience::Public.should_deliver_to(None));
    }

    #[test]
    fn public_is_delivered_to_any_authenticated_user() {
        assert!(Audience::Public.should_deliver_to(Some(1)));
        assert!(Audience::Public.should_deliver_to(Some(999)));
    }

    #[test]
    fn user_audience_is_delivered_to_matching_user_only() {
        assert!(Audience::User(7).should_deliver_to(Some(7)));
    }

    #[test]
    fn user_audience_is_not_delivered_to_a_different_user() {
        assert!(
            !Audience::User(7).should_deliver_to(Some(8)),
            "one user must never receive another user's private order data"
        );
    }

    #[test]
    fn user_audience_is_not_delivered_to_unauthenticated_session() {
        assert!(
            !Audience::User(7).should_deliver_to(None),
            "an unauthenticated socket must never receive any user's private data"
        );
    }

    #[test]
    fn for_user_some_scopes_to_that_user() {
        let env = BroadcastEnvelope::for_user(Some(42), "x".to_string());
        assert_eq!(env.audience, Audience::User(42));
        assert!(env.audience.should_deliver_to(Some(42)));
        assert!(!env.audience.should_deliver_to(Some(43)));
    }

    #[test]
    fn for_user_none_is_public() {
        let env = BroadcastEnvelope::for_user(None, "x".to_string());
        assert_eq!(env.audience, Audience::Public);
    }

    #[test]
    fn public_constructor_sets_public_audience_and_keeps_payload() {
        let env = BroadcastEnvelope::public("hello".to_string());
        assert_eq!(env.audience, Audience::Public);
        assert_eq!(env.payload, "hello");
    }
}
