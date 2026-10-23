//! T01 protocol feasibility probe for the pinned `rmcp` release.
//!
//! A minimal schema-validated fixture tool is driven over raw newline-delimited
//! JSON-RPC (the stdio framing) with both target protocol revisions. This is a
//! conformance probe, not the review engine; the real adapters arrive in T18.
//! Results are recorded in `compat/client-matrix.json`.

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct FixtureEchoInput {
    /// Short fixture text echoed back by the probe tool.
    #[schemars(length(max = 64))]
    text: String,
}

#[derive(Debug, Clone)]
struct ProbeServer;

#[tool_router]
impl ProbeServer {
    #[tool(
        description = "Protocol probe: echo fixture text.",
        annotations(read_only_hint = true)
    )]
    fn fixture_echo(&self, Parameters(input): Parameters<FixtureEchoInput>) -> String {
        format!("echo: {}", input.text)
    }
}

#[tool_handler]
impl ServerHandler for ProbeServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

struct RawClient {
    reader: BufReader<ReadHalf<DuplexStream>>,
    writer: WriteHalf<DuplexStream>,
}

impl RawClient {
    fn start() -> Self {
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            let running = ProbeServer
                .serve(server_io)
                .await
                .expect("probe server starts");
            let _ = running.waiting().await;
        });
        let (read, writer) = tokio::io::split(client_io);
        Self {
            reader: BufReader::new(read),
            writer,
        }
    }

    async fn send(&mut self, message: Value) {
        let mut line = serde_json::to_vec(&message).expect("serialise");
        line.push(b'\n');
        self.writer.write_all(&line).await.expect("write");
    }

    async fn recv(&mut self) -> Value {
        let mut line = String::new();
        let n = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.reader.read_line(&mut line),
        )
        .await
        .expect("server replied within 5s")
        .expect("read");
        assert!(n > 0, "server closed the stream");
        serde_json::from_str(&line).expect("server emitted one JSON message per line")
    }

    async fn request(&mut self, message: Value) -> Value {
        self.send(message).await;
        self.recv().await
    }
}

fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": { "name": "razista-probe", "version": "0.1.0" },
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

#[tokio::test]
async fn modern_2026_discover_list_and_call_round_trip() {
    let mut client = RawClient::start();

    let discover = client
        .request(json!({
            "jsonrpc": "2.0", "id": 1, "method": "server/discover",
            "params": { "_meta": modern_meta() }
        }))
        .await;
    let result = &discover["result"];
    let versions = result["supportedVersions"]
        .as_array()
        .expect("supportedVersions");
    assert!(versions.contains(&json!("2026-07-28")), "{discover}");
    assert!(versions.contains(&json!("2025-11-25")), "{discover}");
    assert_eq!(result["resultType"], "complete", "{discover}");
    assert!(result.get("ttlMs").is_some(), "{discover}");
    assert!(result.get("cacheScope").is_some(), "{discover}");

    let list = client
        .request(json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/list",
            "params": { "_meta": modern_meta() }
        }))
        .await;
    assert_eq!(list["result"]["resultType"], "complete", "{list}");
    let tool = &list["result"]["tools"][0];
    assert_eq!(tool["name"], "fixture_echo", "{list}");
    assert_eq!(
        tool["inputSchema"]["additionalProperties"],
        json!(false),
        "unknown tool inputs must be rejected by the schema: {list}"
    );
    assert_eq!(tool["inputSchema"]["properties"]["text"]["maxLength"], 64);

    let call = client
        .request(json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {
                "_meta": modern_meta(),
                "name": "fixture_echo",
                "arguments": { "text": "hi" }
            }
        }))
        .await;
    assert_eq!(call["result"]["resultType"], "complete", "{call}");
    assert_eq!(call["result"]["content"][0]["text"], "echo: hi", "{call}");
}

#[tokio::test]
async fn modern_2026_request_without_metadata_is_rejected() {
    let mut client = RawClient::start();
    client
        .request(json!({
            "jsonrpc": "2.0", "id": 1, "method": "server/discover",
            "params": { "_meta": modern_meta() }
        }))
        .await;
    let list = client
        .request(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .await;
    assert_eq!(list["error"]["code"], -32602, "{list}");
}

#[tokio::test]
async fn legacy_2025_initialise_list_and_call_round_trip() {
    let mut client = RawClient::start();

    let init = client
        .request(json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "razista-probe-legacy", "version": "0.1.0" }
            }
        }))
        .await;
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25", "{init}");
    client
        .send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .await;

    let list = client
        .request(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .await;
    assert_eq!(list["result"]["tools"][0]["name"], "fixture_echo", "{list}");
    assert!(
        list["result"].get("resultType").is_none(),
        "legacy peers keep the pre-2026 wire shape: {list}"
    );

    let call = client
        .request(json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "fixture_echo", "arguments": { "text": "hi" } }
        }))
        .await;
    assert_eq!(call["result"]["content"][0]["text"], "echo: hi", "{call}");
}

#[tokio::test]
async fn unknown_tool_argument_is_rejected_not_ignored() {
    let mut client = RawClient::start();
    client
        .request(json!({
            "jsonrpc": "2.0", "id": 1, "method": "server/discover",
            "params": { "_meta": modern_meta() }
        }))
        .await;
    let call = client
        .request(json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {
                "_meta": modern_meta(),
                "name": "fixture_echo",
                "arguments": { "text": "hi", "shell": "rm -rf /" }
            }
        }))
        .await;
    let rejected = call.get("error").is_some() || call["result"]["isError"] == json!(true);
    assert!(
        rejected,
        "unknown argument must not be silently accepted: {call}"
    );
    assert_ne!(call["result"]["content"][0]["text"], "echo: hi", "{call}");
}
