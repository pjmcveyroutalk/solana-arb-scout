use crate::rpc_transport;
use reqwest::Client;
use serde_json::{json, Value};
use std::env;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

const DEFAULT_UPSTREAM: &str = "https://api.mainnet-beta.solana.com";
const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:8899";
const MAX_REQUEST_SIZE: usize = 64 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

pub fn run() -> Result<(), String> {
    let listen_addr = env::var("SCOUT_LOCAL_RPC_ADDR").unwrap_or_else(|_| DEFAULT_LISTEN_ADDR.to_owned());
    let upstream = env::var("SCOUT_RPC_UPSTREAM").unwrap_or_else(|_| DEFAULT_UPSTREAM.to_owned());

    ensure_loopback_listener(&listen_addr)?;

    let listener = TcpListener::bind(&listen_addr)
        .map_err(|error| format!("failed to bind Scout local RPC at {listen_addr}: {error}"))?;
    let client = Client::builder()
        .build()
        .map_err(|error| format!("failed to build Scout RPC client: {error}"))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to build Scout RPC runtime: {error}"))?;

    println!("=== SCOUT LOCAL RPC ===");
    println!("listening: http://{listen_addr}");
    println!("upstream:  {upstream}");
    println!("mode:      read-only");
    println!("localhost only: YES");

    for connection in listener.incoming() {
        match connection {
            Ok(mut stream) => {
                if let Err(error) = handle_connection(&mut stream, &runtime, &client, &upstream) {
                    eprintln!("local_rpc_error: {error}");
                }
            }
            Err(error) => eprintln!("local_rpc_accept_error: {error}"),
        }
    }

    Ok(())
}

fn ensure_loopback_listener(listen_addr: &str) -> Result<(), String> {
    let address = listen_addr
        .parse::<SocketAddr>()
        .map_err(|error| format!("invalid SCOUT_LOCAL_RPC_ADDR {listen_addr}: {error}"))?;

    if !address.ip().is_loopback() {
        return Err(format!(
            "SCOUT_LOCAL_RPC_ADDR must remain loopback-only, got {listen_addr}"
        ));
    }

    Ok(())
}

fn handle_connection(
    stream: &mut TcpStream,
    runtime: &tokio::runtime::Runtime,
    client: &Client,
    upstream: &str,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .map_err(|error| format!("failed to set local RPC read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(WRITE_TIMEOUT))
        .map_err(|error| format!("failed to set local RPC write timeout: {error}"))?;

    let body = match read_http_body(stream) {
        Ok(body) => body,
        Err(error) => {
            let response = json!({
                "jsonrpc": "2.0",
                "error": {"code": -32700, "message": error},
                "id": Value::Null
            });
            return send_json(stream, "400 Bad Request", &response);
        }
    };

    let request = match serde_json::from_slice::<Value>(&body) {
        Ok(request) => request,
        Err(error) => {
            let response = json!({
                "jsonrpc": "2.0",
                "error": {"code": -32700, "message": format!("invalid JSON: {error}")},
                "id": Value::Null
            });
            return send_json(stream, "400 Bad Request", &response);
        }
    };

    let response = match validate_request(&request) {
        Ok(method) => {
            println!("local_rpc_request: method={method}");
            match runtime.block_on(rpc_transport::post_json(client, upstream, &request, method)) {
                Ok(response) => response,
                Err(error) => {
                    eprintln!("local_rpc_upstream_error: method={method} error={error}");
                    json_rpc_error(request_id(&request), -32000, "upstream RPC failure")
                }
            }
        }
        Err(response) => response,
    };

    send_json(stream, "200 OK", &response)
}

fn validate_request(request: &Value) -> Result<&str, Value> {
    if !request.is_object() {
        return Err(json_rpc_error(
            Value::Null,
            -32600,
            "batch and non-object requests are not supported by Scout local RPC",
        ));
    }

    let id = request_id(request);
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return Err(json_rpc_error(id, -32600, "missing JSON-RPC method"));
    };

    if !is_read_only_method(method) {
        return Err(json_rpc_error(id, -32601, "method not allowed by Scout read-only RPC"));
    }

    Ok(method)
}

fn request_id(request: &Value) -> Value {
    request.get("id").cloned().unwrap_or(Value::Null)
}

fn is_read_only_method(method: &str) -> bool {
    matches!(
        method,
        "getSlot"
            | "getBlockHeight"
            | "getAccountInfo"
            | "getMultipleAccounts"
            | "getProgramAccounts"
            | "getLatestBlockhash"
            | "getSignatureStatuses"
            | "getTransaction"
            | "getSignaturesForAddress"
            | "getBlock"
            | "getTokenAccountBalance"
            | "getTokenSupply"
            | "getEpochInfo"
            | "getVersion"
            | "getHealth"
            | "simulateTransaction"
    )
}

fn json_rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "error": {"code": code, "message": message},
        "id": id
    })
}

fn read_http_body(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    let mut request = Vec::with_capacity(4096);
    let mut buffer = [0_u8; 2048];
    let mut expected_total = None;
    let mut body_start = None;

    loop {
        let count = stream
            .read(&mut buffer)
            .map_err(|error| format!("failed to read local HTTP request: {error}"))?;

        if count == 0 {
            break;
        }

        request.extend_from_slice(&buffer[..count]);

        if request.len() > MAX_REQUEST_SIZE {
            return Err(format!("request exceeds {MAX_REQUEST_SIZE} byte limit"));
        }

        if expected_total.is_none() {
            if let Some(header_end) = find_bytes(&request, b"\r\n\r\n") {
                if !request.starts_with(b"POST ") {
                    return Err("Scout local RPC accepts HTTP POST only".to_owned());
                }

                let start = header_end + 4;
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = parse_content_length(&headers)
                    .ok_or_else(|| "missing or invalid Content-Length header".to_owned())?;
                let total = start
                    .checked_add(content_length)
                    .ok_or_else(|| "request length overflow".to_owned())?;

                if total > MAX_REQUEST_SIZE {
                    return Err(format!("request exceeds {MAX_REQUEST_SIZE} byte limit"));
                }

                body_start = Some(start);
                expected_total = Some(total);
            }
        }

        if let Some(total) = expected_total {
            if request.len() >= total {
                request.truncate(total);
                break;
            }
        }
    }

    let start = body_start.ok_or_else(|| "incomplete HTTP headers".to_owned())?;
    let total = expected_total.ok_or_else(|| "incomplete HTTP request".to_owned())?;

    if request.len() < total {
        return Err("incomplete HTTP request body".to_owned());
    }

    Ok(request[start..total].to_vec())
}

fn parse_content_length(headers: &str) -> Option<usize> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            value.trim().parse::<usize>().ok()
        } else {
            None
        }
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn send_json(stream: &mut TcpStream, status: &str, value: &Value) -> Result<(), String> {
    let body = serde_json::to_vec(value)
        .map_err(|error| format!("failed to serialize local RPC response: {error}"))?;
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );

    stream
        .write_all(headers.as_bytes())
        .and_then(|_| stream.write_all(&body))
        .and_then(|_| stream.flush())
        .map_err(|error| format!("failed to write local RPC response: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{ensure_loopback_listener, is_read_only_method, parse_content_length, validate_request};
    use serde_json::json;

    #[test]
    fn listener_must_be_loopback() {
        assert!(ensure_loopback_listener("127.0.0.1:8899").is_ok());
        assert!(ensure_loopback_listener("0.0.0.0:8899").is_err());
    }

    #[test]
    fn content_length_is_case_insensitive() {
        assert_eq!(
            parse_content_length("Host: localhost\r\ncontent-length: 42\r\n"),
            Some(42)
        );
    }

    #[test]
    fn allowlist_contains_live_read_methods() {
        assert!(is_read_only_method("getSlot"));
        assert!(is_read_only_method("getMultipleAccounts"));
        assert!(is_read_only_method("simulateTransaction"));
        assert!(!is_read_only_method("unknownMethod"));
    }

    #[test]
    fn unknown_method_is_rejected() {
        let request = json!({"jsonrpc":"2.0","id":7,"method":"unknownMethod"});
        assert!(validate_request(&request).is_err());
    }
}
