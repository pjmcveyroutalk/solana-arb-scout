use reqwest::{
    header::{HeaderMap, RETRY_AFTER},
    Client, StatusCode,
};
use serde_json::Value;
use tokio::time::{sleep, Duration};

const MAX_RPC_429_RETRIES: usize = 2;
const DEFAULT_RPC_429_DELAY: Duration = Duration::from_secs(1);
const MAX_RPC_429_DELAY: Duration = Duration::from_secs(5);

pub async fn post_json(
    client: &Client,
    rpc_url: &str,
    request: &Value,
    label: &str,
) -> Result<Value, String> {
    let mut retry_count = 0usize;

    loop {
        let response = client
            .post(rpc_url)
            .json(request)
            .send()
            .await
            .map_err(|error| format!("{label} RPC request failed: {error}"))?;

        let status = response.status();

        if status == StatusCode::TOO_MANY_REQUESTS && retry_count < MAX_RPC_429_RETRIES {
            let delay = retry_after_delay(response.headers());
            retry_count += 1;

            println!(
                "rpc_rate_limited: label={label} retry={retry_count}/{MAX_RPC_429_RETRIES} delay_ms={}",
                delay.as_millis()
            );

            sleep(delay).await;
            continue;
        }

        if !status.is_success() {
            return Err(format!("{label} RPC returned HTTP status {status}"));
        }

        return response
            .json::<Value>()
            .await
            .map_err(|error| format!("{label} RPC returned invalid JSON: {error}"));
    }
}

fn retry_after_delay(headers: &HeaderMap) -> Duration {
    headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .map(|delay| delay.min(MAX_RPC_429_DELAY))
        .unwrap_or(DEFAULT_RPC_429_DELAY)
}

#[cfg(test)]
mod tests {
    use super::{retry_after_delay, DEFAULT_RPC_429_DELAY, MAX_RPC_429_DELAY, MAX_RPC_429_RETRIES};
    use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};
    use tokio::time::Duration;

    #[test]
    fn retry_policy_is_finite() {
        assert_eq!(MAX_RPC_429_RETRIES, 2);
    }

    #[test]
    fn retry_after_uses_integer_seconds() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("3"));

        assert_eq!(retry_after_delay(&headers), Duration::from_secs(3));
    }

    #[test]
    fn retry_after_is_capped() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("999"));

        assert_eq!(retry_after_delay(&headers), MAX_RPC_429_DELAY);
    }

    #[test]
    fn retry_after_falls_back_when_missing() {
        assert_eq!(retry_after_delay(&HeaderMap::new()), DEFAULT_RPC_429_DELAY);
    }

    #[test]
    fn retry_after_falls_back_when_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("not-a-number"));

        assert_eq!(retry_after_delay(&headers), DEFAULT_RPC_429_DELAY);
    }
}
