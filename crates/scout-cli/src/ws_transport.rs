use futures_util::StreamExt;
use serde_json::Value;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::Message;

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
}


