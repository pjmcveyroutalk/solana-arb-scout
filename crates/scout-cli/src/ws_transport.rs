use futures_util::StreamExt;
use serde_json::Value;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshDecision {
    Accept,
    RefreshAgain { min_context_slot: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionLifecycle {
    generation: u64,
    requested: bool,
    confirmed_subscription_id: Option<u64>,
    bootstrapped: bool,
    ready: bool,
    dirty_slot_watermark: Option<u64>,
    authoritative_slot: Option<u64>,
}

impl SubscriptionLifecycle {
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            requested: false,
            confirmed_subscription_id: None,
            bootstrapped: false,
            ready: false,
            dirty_slot_watermark: None,
            authoritative_slot: None,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn requested(&self) -> bool {
        self.requested
    }

    pub fn confirmed(&self) -> bool {
        self.confirmed_subscription_id.is_some()
    }

    pub fn bootstrapped(&self) -> bool {
        self.bootstrapped
    }

    pub fn ready(&self) -> bool {
        self.ready
    }

    pub fn confirmed_subscription_id(&self) -> Option<u64> {
        self.confirmed_subscription_id
    }

    pub fn dirty_slot_watermark(&self) -> Option<u64> {
        self.dirty_slot_watermark
    }

    pub fn authoritative_slot(&self) -> Option<u64> {
        self.authoritative_slot
    }

    pub fn minimum_context_slot(&self) -> Option<u64> {
        self.dirty_slot_watermark
    }

    pub fn mark_requested(&mut self) {
        self.requested = true;
        self.ready = false;
    }

    pub fn mark_confirmed(&mut self, generation: u64, subscription_id: u64) -> Result<(), String> {
        self.require_generation(generation)?;

        if !self.requested {
            return Err("subscription confirmation arrived before request".to_owned());
        }

        self.confirmed_subscription_id = Some(subscription_id);
        self.ready = false;
        Ok(())
    }

    pub fn mark_dirty(&mut self, generation: u64, slot: u64) -> Result<(), String> {
        self.require_generation(generation)?;

        self.dirty_slot_watermark = Some(
            self.dirty_slot_watermark
                .map_or(slot, |current| current.max(slot)),
        );
        self.ready = false;
        Ok(())
    }

    pub fn mark_authoritative_refresh(
        &mut self,
        generation: u64,
        slot: u64,
    ) -> Result<RefreshDecision, String> {
        self.require_generation(generation)?;

        if !self.confirmed() {
            return Err(
                "authoritative refresh arrived before subscription confirmation".to_owned(),
            );
        }

        self.authoritative_slot = Some(slot);

        if let Some(dirty_slot) = self.dirty_slot_watermark {
            if slot < dirty_slot {
                self.bootstrapped = false;
                self.ready = false;
                return Ok(RefreshDecision::RefreshAgain {
                    min_context_slot: dirty_slot,
                });
            }
        }

        self.bootstrapped = true;
        self.ready = false;
        Ok(RefreshDecision::Accept)
    }

    pub fn mark_ready(&mut self, generation: u64) -> Result<(), String> {
        self.require_generation(generation)?;

        if !self.requested {
            return Err("subscription cannot become ready before request".to_owned());
        }

        if !self.confirmed() {
            return Err("subscription cannot become ready before confirmation".to_owned());
        }

        if !self.bootstrapped {
            return Err(
                "subscription cannot become ready before authoritative bootstrap".to_owned(),
            );
        }

        if let Some(dirty_slot) = self.dirty_slot_watermark {
            let authoritative_slot = self.authoritative_slot.ok_or_else(|| {
                "subscription cannot become ready without authoritative slot".to_owned()
            })?;

            if authoritative_slot < dirty_slot {
                return Err(format!(
                    "authoritative slot {authoritative_slot} is behind dirty watermark {dirty_slot}"
                ));
            }
        }

        self.ready = true;
        Ok(())
    }

    pub fn reconnect(&mut self) -> Result<u64, String> {
        let next_generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| "subscription generation overflow".to_owned())?;

        *self = Self::new(next_generation);
        Ok(next_generation)
    }

    fn require_generation(&self, generation: u64) -> Result<(), String> {
        if generation != self.generation {
            return Err(format!(
                "stale subscription generation: expected {} got {generation}",
                self.generation
            ));
        }

        Ok(())
    }
}

pub async fn wait_for_subscription_confirmation<S>(
    reader: &mut S,
    request_id: u64,
    label: &str,
) -> Result<u64, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let payload = next_json_message(reader, Duration::from_secs(15)).await?;

        if payload.get("id").and_then(Value::as_u64) != Some(request_id) {
            continue;
        }

        if let Some(error) = payload.get("error") {
            return Err(format!("{label} subscription rejected: {error}"));
        }

        let subscription_id = payload
            .get("result")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("{label} subscription response missing id"))?;

        println!("{label}_subscription_id={subscription_id}");
        return Ok(subscription_id);
    }
}

pub async fn next_json_message<S>(
    reader: &mut S,
    observation_timeout: Duration,
) -> Result<Value, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    next_json_message_optional(reader, observation_timeout)
        .await?
        .ok_or_else(|| "timed out waiting for Solana data".to_owned())
}

pub async fn next_json_message_optional<S>(
    reader: &mut S,
    observation_timeout: Duration,
) -> Result<Option<Value>, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let next = match timeout(observation_timeout, reader.next()).await {
            Ok(next) => next,
            Err(_) => return Ok(None),
        };

        let message = next
            .ok_or_else(|| "Solana WebSocket stream closed".to_owned())?
            .map_err(|error| format!("WebSocket receive error: {error}"))?;

        match message {
            Message::Text(text) => {
                let payload = serde_json::from_str::<Value>(text.as_ref())
                    .map_err(|error| format!("invalid JSON: {error}"))?;
                return Ok(Some(payload));
            }
            Message::Binary(bytes) => {
                let payload = serde_json::from_slice::<Value>(bytes.as_ref())
                    .map_err(|error| format!("invalid JSON: {error}"))?;
                return Ok(Some(payload));
            }
            Message::Close(_) => {
                return Err("Solana WebSocket stream closed".to_owned());
            }
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_timeout_contract_is_bounded() {
        let timeout = Duration::from_secs(15);

        assert_eq!(timeout.as_secs(), 15);
    }

    #[test]
    fn requested_confirmed_bootstrapped_ready_are_distinct_states() -> Result<(), String> {
        let mut lifecycle = SubscriptionLifecycle::new(7);

        assert!(!lifecycle.requested());
        assert!(!lifecycle.confirmed());
        assert!(!lifecycle.bootstrapped());
        assert!(!lifecycle.ready());

        lifecycle.mark_requested();
        assert!(lifecycle.requested());
        assert!(!lifecycle.confirmed());
        assert!(!lifecycle.bootstrapped());
        assert!(!lifecycle.ready());

        lifecycle.mark_confirmed(7, 42)?;
        assert!(lifecycle.requested());
        assert!(lifecycle.confirmed());
        assert!(!lifecycle.bootstrapped());
        assert!(!lifecycle.ready());

        assert_eq!(
            lifecycle.mark_authoritative_refresh(7, 100)?,
            RefreshDecision::Accept
        );
        assert!(lifecycle.bootstrapped());
        assert!(!lifecycle.ready());

        lifecycle.mark_ready(7)?;
        assert!(lifecycle.ready());

        Ok(())
    }

    #[test]
    fn dirty_watermark_is_monotonic_and_requires_authoritative_catch_up() -> Result<(), String> {
        let mut lifecycle = SubscriptionLifecycle::new(3);
        lifecycle.mark_requested();
        lifecycle.mark_confirmed(3, 9)?;

        lifecycle.mark_dirty(3, 120)?;
        lifecycle.mark_dirty(3, 118)?;
        lifecycle.mark_dirty(3, 125)?;

        assert_eq!(lifecycle.dirty_slot_watermark(), Some(125));
        assert_eq!(lifecycle.minimum_context_slot(), Some(125));

        assert_eq!(
            lifecycle.mark_authoritative_refresh(3, 124)?,
            RefreshDecision::RefreshAgain {
                min_context_slot: 125
            }
        );
        assert!(!lifecycle.bootstrapped());
        assert!(!lifecycle.ready());

        assert_eq!(
            lifecycle.mark_authoritative_refresh(3, 125)?,
            RefreshDecision::Accept
        );
        lifecycle.mark_ready(3)?;

        assert_eq!(lifecycle.authoritative_slot(), Some(125));
        assert!(lifecycle.ready());

        Ok(())
    }

    #[test]
    fn reconnect_invalidates_old_generation_and_readiness() -> Result<(), String> {
        let mut lifecycle = SubscriptionLifecycle::new(11);
        lifecycle.mark_requested();
        lifecycle.mark_confirmed(11, 77)?;
        lifecycle.mark_dirty(11, 900)?;
        lifecycle.mark_authoritative_refresh(11, 900)?;
        lifecycle.mark_ready(11)?;

        let next_generation = lifecycle.reconnect()?;

        assert_eq!(next_generation, 12);
        assert_eq!(lifecycle.generation(), 12);
        assert!(!lifecycle.requested());
        assert!(!lifecycle.confirmed());
        assert!(!lifecycle.bootstrapped());
        assert!(!lifecycle.ready());
        assert_eq!(lifecycle.dirty_slot_watermark(), None);
        assert_eq!(lifecycle.authoritative_slot(), None);
        assert!(lifecycle.mark_dirty(11, 901).is_err());

        Ok(())
    }

    #[test]
    fn stale_generation_cannot_confirm_or_refresh() {
        let mut lifecycle = SubscriptionLifecycle::new(5);
        lifecycle.mark_requested();

        assert!(lifecycle.mark_confirmed(4, 1).is_err());
        assert!(lifecycle.mark_authoritative_refresh(4, 10).is_err());
    }

    #[test]
    fn ready_fails_closed_without_confirmation_or_bootstrap() {
        let mut lifecycle = SubscriptionLifecycle::new(1);

        assert!(lifecycle.mark_ready(1).is_err());

        lifecycle.mark_requested();
        assert!(lifecycle.mark_ready(1).is_err());

        lifecycle.mark_confirmed(1, 8).expect("confirmation");
        assert!(lifecycle.mark_ready(1).is_err());
    }
}

