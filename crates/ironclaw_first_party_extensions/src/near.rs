//! Read-only NEAR mainnet first-party extension for IronClaw.
//!
//! Mirrors the `web_access` extension structure: a plain `NearExecutor`
//! struct with an async `dispatch` method that routes on `capability_id`,
//! adapted into the host runtime by a shim in `ironclaw_reborn_composition`.
//!
//! All NEAR queries go through the host's `Arc<dyn RuntimeHttpEgress>`
//! (never `reqwest` directly), so policy enforcement, byte accounting, and
//! private-IP denial all apply. RPC target is FastNEAR mainnet; the
//! `near.intents_quote` capability targets the 1Click solver API.

use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::FutureExt as _;
use ironclaw_host_api::{
    CapabilityId, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern, ResourceScope,
    ResourceUsage, RuntimeDispatchErrorKind, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressReasonCode, RuntimeHttpEgressRequest, RuntimeKind,
};
use serde_json::{Value, json};

pub const NEAR_EXTENSION_ID: &str = "near";
pub const NEAR_ACCOUNT_CAPABILITY_ID: &str = "near.account";
pub const NEAR_VIEW_CAPABILITY_ID: &str = "near.view";
pub const NEAR_FT_BALANCES_CAPABILITY_ID: &str = "near.ft_balances";
pub const NEAR_NFTS_CAPABILITY_ID: &str = "near.nfts";
pub const NEAR_TX_STATUS_CAPABILITY_ID: &str = "near.tx_status";
pub const NEAR_INTENTS_QUOTE_CAPABILITY_ID: &str = "near.intents_quote";

const FASTNEAR_RPC_URL: &str = "https://rpc.mainnet.fastnear.com/";
pub const FASTNEAR_RPC_HOST: &str = "rpc.mainnet.fastnear.com";
const INTENTS_QUOTE_URL: &str = "https://1click.chaindefuser.com/v0/quote";
pub const INTENTS_HOST: &str = "1click.chaindefuser.com";

pub const NETWORK_EGRESS_LIMIT: u64 = 2 * 1024 * 1024;
const RESPONSE_BODY_LIMIT: u64 = 2 * 1024 * 1024;
const DEFAULT_TIMEOUT_MS: u32 = 30_000;

const MAX_ACCOUNT_ID_CHARS: usize = 64;
const MAX_METHOD_NAME_CHARS: usize = 128;
const MAX_FT_CONTRACTS: usize = 20;
const MAX_TX_HASH_CHARS: usize = 128;
const MAX_FROM_INDEX_CHARS: usize = 64;
const DEFAULT_NFT_LIMIT: u64 = 50;
const MAX_NFT_LIMIT: u64 = 100;
/// 1Click `slippageTolerance` is expressed in basis points (100 = 1%). The
/// schema and API cap this at 10000 bp (100%).
const DEFAULT_SLIPPAGE_TOLERANCE: u64 = 100;
const MAX_SLIPPAGE_TOLERANCE: u64 = 10_000;
/// Allowed `swapType` values per the 1Click `/v0/quote` API.
const ALLOWED_SWAP_TYPES: [&str; 2] = ["EXACT_INPUT", "EXACT_OUTPUT"];

#[derive(Debug, Default)]
pub struct NearExecutor {}

pub struct NearDispatchRequest<'a> {
    pub capability_id: &'a CapabilityId,
    pub scope: &'a ResourceScope,
    pub input: &'a Value,
    pub runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearDispatchResult {
    pub output: Value,
    pub usage: ResourceUsage,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("near dispatch failed: {kind}")]
pub struct NearDispatchError {
    kind: RuntimeDispatchErrorKind,
    usage: Option<ResourceUsage>,
}

impl NearDispatchError {
    fn new(kind: RuntimeDispatchErrorKind) -> Self {
        Self { kind, usage: None }
    }

    fn with_usage(mut self, usage: ResourceUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        self.kind
    }

    pub fn usage(&self) -> Option<&ResourceUsage> {
        self.usage.as_ref()
    }
}

impl NearExecutor {
    pub async fn dispatch(
        &self,
        request: NearDispatchRequest<'_>,
    ) -> Result<NearDispatchResult, NearDispatchError> {
        match request.capability_id.as_str() {
            NEAR_ACCOUNT_CAPABILITY_ID => self.account(request).await,
            NEAR_VIEW_CAPABILITY_ID => self.view(request).await,
            NEAR_FT_BALANCES_CAPABILITY_ID => self.ft_balances(request).await,
            NEAR_NFTS_CAPABILITY_ID => self.nfts(request).await,
            NEAR_TX_STATUS_CAPABILITY_ID => self.tx_status(request).await,
            NEAR_INTENTS_QUOTE_CAPABILITY_ID => self.intents_quote(request).await,
            _ => Err(NearDispatchError::new(
                RuntimeDispatchErrorKind::UndeclaredCapability,
            )),
        }
    }

    async fn account(
        &self,
        request: NearDispatchRequest<'_>,
    ) -> Result<NearDispatchResult, NearDispatchError> {
        let egress = require_egress(&request)?;
        let account_id = required_account_id(request.input, "account_id")?;

        let rpc = json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "query",
            "params": {
                "request_type": "view_account",
                "finality": "final",
                "account_id": account_id,
            }
        });
        let (body, egress_bytes) = http_post_json(
            &request,
            egress,
            near_rpc_network_policy(),
            FASTNEAR_RPC_URL,
            &rpc,
        )
        .await?;
        let result = rpc_result(&body, egress_bytes)?;
        let output = json!({
            "amount": result["amount"],
            "locked": result["locked"],
            "code_hash": result["code_hash"],
            "storage_usage": result["storage_usage"],
            "block_height": result["block_height"],
        });
        Ok(success(output, egress_bytes))
    }

    async fn view(
        &self,
        request: NearDispatchRequest<'_>,
    ) -> Result<NearDispatchResult, NearDispatchError> {
        let egress = require_egress(&request)?;
        let account_id = required_account_id(request.input, "account_id")?;
        let method_name = required_method_name(request.input, "method_name")?;
        let args = optional_object(request.input, "args")?;

        let body =
            call_function(&request, egress, &account_id, &method_name, args.as_ref()).await?;
        let egress_bytes = body.egress_bytes;
        let parsed = decode_view_result(&body.value, egress_bytes)?;
        let output = json!({
            "result": parsed,
            "block_height": body.value["result"]["block_height"],
        });
        Ok(success(output, egress_bytes))
    }

    async fn ft_balances(
        &self,
        request: NearDispatchRequest<'_>,
    ) -> Result<NearDispatchResult, NearDispatchError> {
        let egress = require_egress(&request)?;
        let account_id = required_account_id(request.input, "account_id")?;
        let token_contracts = required_string_array(
            request.input,
            "token_contracts",
            MAX_FT_CONTRACTS,
            MAX_ACCOUNT_ID_CHARS,
        )?;

        let args = json!({ "account_id": account_id });
        let request_ref = &request;
        let args_ref = &args;
        // Fan the per-contract balance reads out concurrently; a serial loop
        // would pay one RPC round-trip per token. The first failure aborts the
        // batch (try_join_all short-circuits), matching the previous behavior.
        let lookups = token_contracts.into_iter().map(|contract| {
            let egress = Arc::clone(&egress);
            async move {
                let body = call_function(
                    request_ref,
                    egress,
                    &contract,
                    "ft_balance_of",
                    Some(args_ref),
                )
                .await?;
                let parsed = decode_view_result(&body.value, body.egress_bytes)?;
                // NEP-141 ft_balance_of returns a quoted integer string.
                Ok::<_, NearDispatchError>((contract, parsed, body.egress_bytes))
            }
        });
        let results = futures_util::future::try_join_all(lookups).await?;

        let mut total_egress_bytes = 0_u64;
        let mut balances = Vec::with_capacity(results.len());
        for (contract, parsed, egress_bytes) in results {
            total_egress_bytes = total_egress_bytes.saturating_add(egress_bytes);
            balances.push(json!({
                "contract": contract,
                "raw": parsed,
            }));
        }

        let output = json!({ "balances": balances });
        Ok(success(output, total_egress_bytes))
    }

    async fn nfts(
        &self,
        request: NearDispatchRequest<'_>,
    ) -> Result<NearDispatchResult, NearDispatchError> {
        let egress = require_egress(&request)?;
        let account_id = required_account_id(request.input, "account_id")?;
        let nft_contract = required_account_id(request.input, "nft_contract")?;
        let from_index = match optional_string(request.input, "from_index")? {
            Some(value) => {
                if value.chars().count() > MAX_FROM_INDEX_CHARS {
                    return Err(input_error());
                }
                value
            }
            None => "0".to_string(),
        };
        let limit = optional_u64(request.input, "limit")?
            .unwrap_or(DEFAULT_NFT_LIMIT)
            .clamp(1, MAX_NFT_LIMIT);

        let args = json!({
            "account_id": account_id,
            "from_index": from_index,
            "limit": limit,
        });
        let body = call_function(
            &request,
            egress,
            &nft_contract,
            "nft_tokens_for_owner",
            Some(&args),
        )
        .await?;
        let egress_bytes = body.egress_bytes;
        let parsed = decode_view_result(&body.value, egress_bytes)?;
        let output = json!({ "tokens": parsed });
        Ok(success(output, egress_bytes))
    }

    async fn tx_status(
        &self,
        request: NearDispatchRequest<'_>,
    ) -> Result<NearDispatchResult, NearDispatchError> {
        let egress = require_egress(&request)?;
        let tx_hash = required_bounded(request.input, "tx_hash", MAX_TX_HASH_CHARS)?;
        let sender_account_id = required_account_id(request.input, "sender_account_id")?;

        let rpc = json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "EXPERIMENTAL_tx_status",
            "params": {
                "tx_hash": tx_hash,
                "sender_account_id": sender_account_id,
            }
        });
        let (body, egress_bytes) = http_post_json(
            &request,
            egress,
            near_rpc_network_policy(),
            FASTNEAR_RPC_URL,
            &rpc,
        )
        .await?;
        let result = rpc_result(&body, egress_bytes)?;
        let output = json!({
            "status": result["status"],
            "receipts_outcome": result["receipts_outcome"],
            "transaction": result["transaction"],
        });
        Ok(success(output, egress_bytes))
    }

    async fn intents_quote(
        &self,
        request: NearDispatchRequest<'_>,
    ) -> Result<NearDispatchResult, NearDispatchError> {
        let egress = require_egress(&request)?;
        let origin_asset = required_bounded(request.input, "origin_asset", MAX_METHOD_NAME_CHARS)?;
        let destination_asset =
            required_bounded(request.input, "destination_asset", MAX_METHOD_NAME_CHARS)?;
        let amount = required_bounded(request.input, "amount", MAX_METHOD_NAME_CHARS)?;
        let recipient = required_bounded(request.input, "recipient", MAX_METHOD_NAME_CHARS)?;
        let refund_to = required_account_id(request.input, "refund_to")?;
        let swap_type =
            optional_string(request.input, "swap_type")?.unwrap_or_else(|| "EXACT_INPUT".into());
        if !ALLOWED_SWAP_TYPES.contains(&swap_type.as_str()) {
            return Err(input_error());
        }
        // Slippage is expressed in basis points; clamp to a ceiling so a caller
        // can't request a degenerate 100%+ tolerance.
        let slippage_tolerance = optional_u64(request.input, "slippage_tolerance")?
            .unwrap_or(DEFAULT_SLIPPAGE_TOLERANCE)
            .min(MAX_SLIPPAGE_TOLERANCE);

        // `dry: true` is forced — this is a read-only quote capability and must
        // never request execution.
        let payload = json!({
            "swapType": swap_type,
            "originAsset": origin_asset,
            "destinationAsset": destination_asset,
            "amount": amount,
            "depositType": "INTENTS",
            "recipientType": "DESTINATION_CHAIN",
            "recipient": recipient,
            "refundTo": refund_to,
            "refundType": "INTENTS",
            "slippageTolerance": slippage_tolerance,
            "dry": true,
        });
        let (body, egress_bytes) = http_post_json(
            &request,
            egress,
            intents_network_policy(),
            INTENTS_QUOTE_URL,
            &payload,
        )
        .await?;
        // The 1Click quote response nests its payload under `quote`. A response
        // without it is an error envelope or an unexpected shape, not a quote, so
        // surface a failure rather than silently returning null fields.
        let quote = body
            .get("quote")
            .ok_or_else(|| operation_error(egress_bytes))?;
        let output = json!({
            "amount_out": quote["amountOut"],
            "deposit_address": quote["depositAddress"],
            "fee": quote["fee"],
            "deadline": quote["deadline"],
        });
        Ok(success(output, egress_bytes))
    }
}

/// Result of a `call_function` query: the raw RPC body plus bytes spent.
struct CallFunctionResponse {
    value: Value,
    egress_bytes: u64,
}

fn require_egress(
    request: &NearDispatchRequest<'_>,
) -> Result<Arc<dyn RuntimeHttpEgress>, NearDispatchError> {
    request
        .runtime_http_egress
        .as_ref()
        .ok_or_else(|| NearDispatchError::new(RuntimeDispatchErrorKind::NetworkDenied))
        .cloned()
}

/// POST an arbitrary JSON payload, returning the parsed response body and the
/// total bytes spent. HTTP-level errors map through `map_egress_error`.
async fn http_post_json(
    request: &NearDispatchRequest<'_>,
    egress: Arc<dyn RuntimeHttpEgress>,
    network_policy: NetworkPolicy,
    url: &str,
    payload: &Value,
) -> Result<(Value, u64), NearDispatchError> {
    let body = serde_json::to_vec(payload).map_err(|_| input_error())?;
    let http = RuntimeHttpEgressRequest {
        runtime: RuntimeKind::FirstParty,
        scope: request.scope.clone(),
        capability_id: request.capability_id.clone(),
        method: NetworkMethod::Post,
        url: url.to_string(),
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body,
        network_policy,
        credential_injections: Vec::new(),
        response_body_limit: Some(RESPONSE_BODY_LIMIT),
        save_body_to: None,
        timeout_ms: Some(DEFAULT_TIMEOUT_MS),
    };
    let resp = execute_runtime_http(http, egress)
        .await
        .map_err(map_egress_error)?;
    let egress_bytes = resp.request_bytes.saturating_add(resp.response_bytes);
    let parsed: Value =
        serde_json::from_slice(&resp.body).map_err(|_| output_decode_error(egress_bytes))?;
    Ok((parsed, egress_bytes))
}

/// Issue a `call_function` view query against `account_id`, base64-encoding the
/// JSON args. Checks the RPC `error` branch before returning.
async fn call_function(
    request: &NearDispatchRequest<'_>,
    egress: Arc<dyn RuntimeHttpEgress>,
    account_id: &str,
    method_name: &str,
    args: Option<&Value>,
) -> Result<CallFunctionResponse, NearDispatchError> {
    let args_base64 = encode_args(args)?;
    let rpc = json!({
        "jsonrpc": "2.0",
        "id": "1",
        "method": "query",
        "params": {
            "request_type": "call_function",
            "finality": "final",
            "account_id": account_id,
            "method_name": method_name,
            "args_base64": args_base64,
        }
    });
    let (body, egress_bytes) = http_post_json(
        request,
        egress,
        near_rpc_network_policy(),
        FASTNEAR_RPC_URL,
        &rpc,
    )
    .await?;
    if body.get("error").is_some() {
        return Err(operation_error(egress_bytes));
    }
    Ok(CallFunctionResponse {
        value: body,
        egress_bytes,
    })
}

/// base64-encode the JSON-serialized args object. `None`/null → empty string.
fn encode_args(args: Option<&Value>) -> Result<String, NearDispatchError> {
    match args {
        None => Ok(String::new()),
        Some(Value::Null) => Ok(String::new()),
        Some(value) => {
            let bytes = serde_json::to_vec(value).map_err(|_| input_error())?;
            Ok(STANDARD.encode(bytes))
        }
    }
}

/// Decode a `call_function` RPC body: `result.result` is a raw byte array (NOT
/// base64) that we convert directly to UTF-8 and JSON-parse.
/// Construct an `OutputDecode` error annotated with the bytes already spent on
/// the round-trip that produced the undecodable body.
fn output_decode_error(egress_bytes: u64) -> NearDispatchError {
    NearDispatchError::new(RuntimeDispatchErrorKind::OutputDecode).with_usage(ResourceUsage {
        network_egress_bytes: egress_bytes,
        ..ResourceUsage::default()
    })
}

fn decode_view_result(body: &Value, egress_bytes: u64) -> Result<Value, NearDispatchError> {
    if body.get("error").is_some() {
        return Err(operation_error(egress_bytes));
    }
    let raw = body
        .pointer("/result/result")
        .ok_or_else(|| output_decode_error(egress_bytes))?;
    let bytes: Vec<u8> =
        serde_json::from_value(raw.clone()).map_err(|_| output_decode_error(egress_bytes))?;
    // A view method returning nothing yields an empty byte array; surface it as
    // JSON null instead of failing to parse an empty slice.
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|_| output_decode_error(egress_bytes))
}

/// Extract the `result` object from a top-level query RPC body, mapping an RPC
/// `error` to `OperationFailed`.
fn rpc_result(body: &Value, egress_bytes: u64) -> Result<Value, NearDispatchError> {
    if body.get("error").is_some() {
        return Err(operation_error(egress_bytes));
    }
    body.get("result")
        .cloned()
        .ok_or_else(|| output_decode_error(egress_bytes))
}

fn success(output: Value, egress_bytes: u64) -> NearDispatchResult {
    let output_bytes = serde_json::to_vec(&output)
        .map(|bytes| bytes.len() as u64)
        .unwrap_or(0);
    NearDispatchResult {
        output,
        usage: ResourceUsage {
            output_bytes,
            network_egress_bytes: egress_bytes,
            ..ResourceUsage::default()
        },
    }
}

async fn execute_runtime_http(
    request: RuntimeHttpEgressRequest,
    egress: Arc<dyn RuntimeHttpEgress>,
) -> Result<ironclaw_host_api::RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
    std::panic::AssertUnwindSafe(egress.execute(request))
        .catch_unwind()
        .await
        .map_err(|_| RuntimeHttpEgressError::Network {
            reason: "worker_join".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        })?
}

fn near_rpc_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: FASTNEAR_RPC_HOST.to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(NETWORK_EGRESS_LIMIT),
    }
}

fn intents_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: INTENTS_HOST.to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(NETWORK_EGRESS_LIMIT),
    }
}

fn map_egress_error(error: RuntimeHttpEgressError) -> NearDispatchError {
    let kind = match error.reason_code() {
        RuntimeHttpEgressReasonCode::CredentialUnavailable => RuntimeDispatchErrorKind::Client,
        RuntimeHttpEgressReasonCode::RequestDenied => RuntimeDispatchErrorKind::InputEncode,
        RuntimeHttpEgressReasonCode::PolicyDenied => RuntimeDispatchErrorKind::PolicyDenied,
        RuntimeHttpEgressReasonCode::NetworkError => RuntimeDispatchErrorKind::NetworkDenied,
        RuntimeHttpEgressReasonCode::ResponseError => RuntimeDispatchErrorKind::OutputDecode,
        RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded => {
            RuntimeDispatchErrorKind::OutputTooLarge
        }
    };
    NearDispatchError::new(kind).with_usage(ResourceUsage {
        network_egress_bytes: error.request_bytes(),
        ..ResourceUsage::default()
    })
}

fn input_error() -> NearDispatchError {
    NearDispatchError::new(RuntimeDispatchErrorKind::InputEncode)
}

fn operation_error(egress_bytes: u64) -> NearDispatchError {
    NearDispatchError::new(RuntimeDispatchErrorKind::OperationFailed).with_usage(ResourceUsage {
        network_egress_bytes: egress_bytes,
        ..ResourceUsage::default()
    })
}

fn required_account_id(input: &Value, key: &str) -> Result<String, NearDispatchError> {
    required_bounded(input, key, MAX_ACCOUNT_ID_CHARS)
}

fn required_method_name(input: &Value, key: &str) -> Result<String, NearDispatchError> {
    required_bounded(input, key, MAX_METHOD_NAME_CHARS)
}

fn required_bounded(
    input: &Value,
    key: &str,
    max_chars: usize,
) -> Result<String, NearDispatchError> {
    let value = input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(input_error)?;
    if value.chars().count() > max_chars {
        return Err(input_error());
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(input_error());
    }
    Ok(trimmed.to_string())
}

fn optional_string(input: &Value, key: &str) -> Result<Option<String>, NearDispatchError> {
    let Some(value) = input.get(key) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|value| Some(value.to_string()))
        .ok_or_else(input_error)
}

fn optional_u64(input: &Value, key: &str) -> Result<Option<u64>, NearDispatchError> {
    let Some(value) = input.get(key) else {
        return Ok(None);
    };
    value.as_u64().map(Some).ok_or_else(input_error)
}

fn optional_object(input: &Value, key: &str) -> Result<Option<Value>, NearDispatchError> {
    let Some(value) = input.get(key) else {
        return Ok(None);
    };
    if value.is_object() {
        Ok(Some(value.clone()))
    } else {
        Err(input_error())
    }
}

fn required_string_array(
    input: &Value,
    key: &str,
    max_items: usize,
    max_chars: usize,
) -> Result<Vec<String>, NearDispatchError> {
    let values = input
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(input_error)?;
    if values.is_empty() || values.len() > max_items {
        return Err(input_error());
    }
    values
        .iter()
        .map(|item| {
            let value = item.as_str().ok_or_else(input_error)?;
            if value.chars().count() > max_chars {
                return Err(input_error());
            }
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Err(input_error());
            }
            Ok(trimmed.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{InvocationId, RuntimeHttpEgressResponse, UserId};
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;

    fn scope() -> ResourceScope {
        ResourceScope::local_default(UserId::new("test-user").unwrap(), InvocationId::new())
            .unwrap()
    }

    fn capability_id(value: &str) -> CapabilityId {
        CapabilityId::new(value).unwrap()
    }

    fn request<'a>(
        capability_id: &'a CapabilityId,
        scope: &'a ResourceScope,
        input: &'a Value,
        runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    ) -> NearDispatchRequest<'a> {
        NearDispatchRequest {
            capability_id,
            scope,
            input,
            runtime_http_egress,
        }
    }

    struct RecordingEgress {
        responses: StdMutex<VecDeque<Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>>>,
        requests: StdMutex<Vec<Value>>,
    }

    impl RecordingEgress {
        fn ok_json(body: Value) -> RuntimeHttpEgressResponse {
            let bytes = serde_json::to_vec(&body).unwrap();
            RuntimeHttpEgressResponse {
                status: 200,
                headers: Vec::new(),
                body: bytes,
                saved_body: None,
                request_bytes: 10,
                response_bytes: 20,
                redaction_applied: false,
            }
        }

        fn single(body: Value) -> Self {
            Self::queued(vec![body])
        }

        /// Queue several successful JSON responses, popped in call order.
        fn queued(bodies: Vec<Value>) -> Self {
            Self {
                responses: StdMutex::new(
                    bodies.into_iter().map(|b| Ok(Self::ok_json(b))).collect(),
                ),
                requests: StdMutex::new(Vec::new()),
            }
        }

        /// The JSON request bodies seen by `execute`, in call order.
        fn recorded_requests(&self) -> Vec<Value> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl RuntimeHttpEgress for RecordingEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            let body = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            self.requests.lock().unwrap().push(body);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("RecordingEgress: no more responses queued")
        }
    }

    /// Build a `call_function` RPC body whose `result.result` byte array decodes
    /// to the given JSON string.
    fn call_function_body(json_str: &str) -> Value {
        let bytes: Vec<u8> = json_str.as_bytes().to_vec();
        json!({
            "jsonrpc": "2.0",
            "id": "1",
            "result": {
                "result": bytes,
                "logs": [],
                "block_height": 12_345_678_u64,
                "block_hash": "abc",
            }
        })
    }

    // ----- pure-function decode tests -----

    #[test]
    fn encode_args_empty_for_none_and_null() {
        assert_eq!(encode_args(None).unwrap(), "");
        assert_eq!(encode_args(Some(&Value::Null)).unwrap(), "");
    }

    #[test]
    fn encode_args_matches_known_base64() {
        // {"account_id":"alice.near"} → eyJhY2NvdW50X2lkIjoiYWxpY2UubmVhciJ9
        let args = json!({ "account_id": "alice.near" });
        assert_eq!(
            encode_args(Some(&args)).unwrap(),
            "eyJhY2NvdW50X2lkIjoiYWxpY2UubmVhciJ9"
        );
    }

    #[test]
    fn decode_view_result_parses_object_body() {
        let body = call_function_body(r#"{"name":"Wrapped NEAR","decimals":24}"#);
        let parsed = decode_view_result(&body, 0).unwrap();
        assert_eq!(parsed["name"], "Wrapped NEAR");
        assert_eq!(parsed["decimals"], 24);
    }

    #[test]
    fn decode_view_result_parses_ft_balance_quoted_string() {
        // ft_balance_of returns a quoted integer string.
        let body = call_function_body(r#""1000000""#);
        let parsed = decode_view_result(&body, 0).unwrap();
        assert_eq!(parsed, Value::String("1000000".to_string()));
    }

    #[test]
    fn decode_view_result_maps_rpc_error_to_operation_failed() {
        let body = json!({"jsonrpc":"2.0","id":"1","error":{"name":"UNKNOWN_ACCOUNT"}});
        let err = decode_view_result(&body, 42).unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OperationFailed);
        assert_eq!(err.usage().unwrap().network_egress_bytes, 42);
    }

    #[test]
    fn decode_view_result_rejects_non_byte_array() {
        let body = json!({"result":{"result":"not-an-array"}});
        let err = decode_view_result(&body, 0).unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OutputDecode);
    }

    // ----- async end-to-end tests -----

    #[tokio::test]
    async fn dispatch_returns_undeclared_capability_for_unknown_id() {
        let executor = NearExecutor::default();
        let capability = capability_id("near.unknown");
        let scope = scope();
        let input = json!({});

        let error = executor
            .dispatch(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::UndeclaredCapability);
    }

    #[tokio::test]
    async fn account_returns_network_denied_when_egress_is_none() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_ACCOUNT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"account_id":"alice.near"});

        let error = executor
            .dispatch(request(&capability, &scope, &input, None))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::NetworkDenied);
    }

    #[tokio::test]
    async fn account_rejects_missing_account_id() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_ACCOUNT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({});
        let egress = Arc::new(RecordingEgress::single(json!({})));

        let error = executor
            .dispatch(request(&capability, &scope, &input, Some(egress)))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
    }

    #[tokio::test]
    async fn account_happy_path_returns_balance_fields() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_ACCOUNT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"account_id":"alice.near"});
        let rpc_body = json!({
            "jsonrpc": "2.0",
            "id": "1",
            "result": {
                "amount": "100000000000000000000000000",
                "locked": "0",
                "code_hash": "11111111111111111111111111111111",
                "storage_usage": 182,
                "block_height": 12_345_678_u64,
                "block_hash": "xyz",
            }
        });
        let egress = Arc::new(RecordingEgress::single(rpc_body));

        let result = executor
            .dispatch(request(&capability, &scope, &input, Some(egress)))
            .await
            .unwrap();

        assert_eq!(result.output["amount"], "100000000000000000000000000");
        assert_eq!(result.output["storage_usage"], 182);
        assert_eq!(result.output["block_height"], 12_345_678_u64);
        assert!(result.usage.network_egress_bytes > 0);
    }

    #[tokio::test]
    async fn account_maps_rpc_error_to_operation_failed() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_ACCOUNT_CAPABILITY_ID);
        let scope = scope();
        let input = json!({"account_id":"ghost.near"});
        let rpc_body = json!({
            "jsonrpc": "2.0",
            "id": "1",
            "error": {"name": "UNKNOWN_ACCOUNT", "cause": {"name": "UNKNOWN_ACCOUNT"}}
        });
        let egress = Arc::new(RecordingEgress::single(rpc_body));

        let error = executor
            .dispatch(request(&capability, &scope, &input, Some(egress)))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::OperationFailed);
        assert!(error.usage().unwrap().network_egress_bytes > 0);
    }

    #[tokio::test]
    async fn view_happy_path_decodes_contract_result() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_VIEW_CAPABILITY_ID);
        let scope = scope();
        let input = json!({
            "account_id": "token.v2.ref-finance.near",
            "method_name": "ft_metadata",
        });
        let egress = Arc::new(RecordingEgress::single(call_function_body(
            r#"{"spec":"ft-1.0.0","name":"Ref","symbol":"REF","decimals":18}"#,
        )));

        let result = executor
            .dispatch(request(&capability, &scope, &input, Some(egress)))
            .await
            .unwrap();

        assert_eq!(result.output["result"]["symbol"], "REF");
        assert_eq!(result.output["result"]["decimals"], 18);
        assert_eq!(result.output["block_height"], 12_345_678_u64);
        assert!(result.usage.network_egress_bytes > 0);
    }

    #[tokio::test]
    async fn view_rejects_non_object_args() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_VIEW_CAPABILITY_ID);
        let scope = scope();
        let input = json!({
            "account_id": "x.near",
            "method_name": "foo",
            "args": "not-an-object",
        });
        let egress = Arc::new(RecordingEgress::single(json!({})));

        let error = executor
            .dispatch(request(&capability, &scope, &input, Some(egress)))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
    }

    #[tokio::test]
    async fn ft_balances_decodes_quoted_balances() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_FT_BALANCES_CAPABILITY_ID);
        let scope = scope();
        let input = json!({
            "account_id": "alice.near",
            "token_contracts": ["usdt.tether-token.near"],
        });
        let egress = Arc::new(RecordingEgress::single(call_function_body(r#""1000000""#)));

        let result = executor
            .dispatch(request(&capability, &scope, &input, Some(egress)))
            .await
            .unwrap();

        assert_eq!(
            result.output["balances"][0]["contract"],
            "usdt.tether-token.near"
        );
        assert_eq!(result.output["balances"][0]["raw"], "1000000");
    }

    #[test]
    fn decode_view_result_empty_bytes_returns_null() {
        // A view method that returns nothing yields an empty byte array; we
        // surface JSON null rather than failing to parse an empty slice.
        let body = json!({ "result": { "result": [] } });
        assert_eq!(decode_view_result(&body, 0).unwrap(), Value::Null);
    }

    #[test]
    fn decode_view_result_missing_result_pointer_is_output_decode() {
        let body = json!({ "result": { "logs": [] } });
        let err = decode_view_result(&body, 7).unwrap_err();
        assert_eq!(err.kind(), RuntimeDispatchErrorKind::OutputDecode);
        assert_eq!(err.usage().unwrap().network_egress_bytes, 7);
    }

    #[tokio::test]
    async fn ft_balances_rejects_empty_contract_list() {
        // A balance query over zero contracts is meaningless; required_string_array
        // rejects it before any RPC round-trip happens.
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_FT_BALANCES_CAPABILITY_ID);
        let scope = scope();
        let input = json!({ "account_id": "alice.near", "token_contracts": [] });
        let egress = Arc::new(RecordingEgress::queued(vec![]));

        let error = executor
            .dispatch(request(&capability, &scope, &input, Some(egress.clone())))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
        assert!(egress.recorded_requests().is_empty());
    }

    #[tokio::test]
    async fn ft_balances_preserves_contract_order_when_concurrent() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_FT_BALANCES_CAPABILITY_ID);
        let scope = scope();
        let input = json!({
            "account_id": "alice.near",
            "token_contracts": ["a.near", "b.near"],
        });
        let egress = Arc::new(RecordingEgress::queued(vec![
            call_function_body(r#""111""#),
            call_function_body(r#""222""#),
        ]));

        let result = executor
            .dispatch(request(&capability, &scope, &input, Some(egress)))
            .await
            .unwrap();

        // try_join_all preserves input order regardless of completion order.
        let balances = result.output["balances"].as_array().unwrap();
        assert_eq!(balances.len(), 2);
        assert_eq!(balances[0]["contract"], "a.near");
        assert_eq!(balances[1]["contract"], "b.near");
    }

    #[tokio::test]
    async fn nfts_happy_path_returns_tokens() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_NFTS_CAPABILITY_ID);
        let scope = scope();
        let input = json!({ "account_id": "alice.near", "nft_contract": "nft.near" });
        let egress = Arc::new(RecordingEgress::single(call_function_body(
            r#"[{"token_id":"1","owner_id":"alice.near"}]"#,
        )));

        let result = executor
            .dispatch(request(&capability, &scope, &input, Some(egress)))
            .await
            .unwrap();

        assert_eq!(result.output["tokens"][0]["token_id"], "1");
        assert!(result.usage.network_egress_bytes > 0);
    }

    #[tokio::test]
    async fn nfts_rejects_overlong_from_index() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_NFTS_CAPABILITY_ID);
        let scope = scope();
        let from_index = "1".repeat(MAX_FROM_INDEX_CHARS + 1);
        let input = json!({
            "account_id": "alice.near",
            "nft_contract": "nft.near",
            "from_index": from_index,
        });
        let egress = Arc::new(RecordingEgress::single(json!({})));

        let error = executor
            .dispatch(request(&capability, &scope, &input, Some(egress)))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
    }

    #[tokio::test]
    async fn tx_status_happy_path_returns_transaction() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_TX_STATUS_CAPABILITY_ID);
        let scope = scope();
        let input = json!({
            "tx_hash": "11111111111111111111111111111111",
            "sender_account_id": "alice.near",
        });
        let rpc_body = json!({
            "jsonrpc": "2.0",
            "id": "1",
            "result": {
                "status": { "SuccessValue": "" },
                "transaction": { "hash": "abc", "signer_id": "alice.near" },
                "receipts_outcome": [],
            }
        });
        let egress = Arc::new(RecordingEgress::single(rpc_body));

        let result = executor
            .dispatch(request(&capability, &scope, &input, Some(egress)))
            .await
            .unwrap();

        assert_eq!(result.output["transaction"]["signer_id"], "alice.near");
        assert!(result.usage.network_egress_bytes > 0);
    }

    #[tokio::test]
    async fn tx_status_maps_rpc_error_to_operation_failed() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_TX_STATUS_CAPABILITY_ID);
        let scope = scope();
        let input = json!({ "tx_hash": "deadbeef", "sender_account_id": "alice.near" });
        let rpc_body = json!({"jsonrpc":"2.0","id":"1","error":{"name":"UNKNOWN_TRANSACTION"}});
        let egress = Arc::new(RecordingEgress::single(rpc_body));

        let error = executor
            .dispatch(request(&capability, &scope, &input, Some(egress)))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::OperationFailed);
    }

    fn intents_input() -> Value {
        json!({
            "origin_asset": "nep141:wrap.near",
            "destination_asset": "nep141:usdt.tether-token.near",
            "amount": "1000000",
            "recipient": "0xabc",
            "refund_to": "alice.near",
        })
    }

    fn intents_quote_body() -> Value {
        json!({
            "quote": {
                "amountOut": "990000",
                "depositAddress": "intents.near",
                "fee": "1000",
                "deadline": "2026-01-01T00:00:00Z",
            }
        })
    }

    #[tokio::test]
    async fn intents_quote_forces_dry_and_returns_fields() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_INTENTS_QUOTE_CAPABILITY_ID);
        let scope = scope();
        let input = intents_input();
        let egress = Arc::new(RecordingEgress::single(intents_quote_body()));

        let result = executor
            .dispatch(request(&capability, &scope, &input, Some(egress.clone())))
            .await
            .unwrap();

        assert_eq!(result.output["amount_out"], "990000");
        assert_eq!(result.output["deposit_address"], "intents.near");
        // `dry: true` must always be sent — this is a read-only quote capability.
        let sent = egress.recorded_requests();
        assert_eq!(sent[0]["dry"], true);
        assert_eq!(sent[0]["swapType"], "EXACT_INPUT");
    }

    #[tokio::test]
    async fn intents_quote_rejects_unknown_swap_type() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_INTENTS_QUOTE_CAPABILITY_ID);
        let scope = scope();
        let mut input = intents_input();
        input["swap_type"] = json!("LIMIT_ORDER");
        let egress = Arc::new(RecordingEgress::single(json!({})));

        let error = executor
            .dispatch(request(&capability, &scope, &input, Some(egress)))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
    }

    #[tokio::test]
    async fn intents_quote_clamps_slippage_to_ceiling() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_INTENTS_QUOTE_CAPABILITY_ID);
        let scope = scope();
        let mut input = intents_input();
        input["slippage_tolerance"] = json!(999_999);
        let egress = Arc::new(RecordingEgress::single(intents_quote_body()));

        executor
            .dispatch(request(&capability, &scope, &input, Some(egress.clone())))
            .await
            .unwrap();

        let sent = egress.recorded_requests();
        assert_eq!(sent[0]["slippageTolerance"], MAX_SLIPPAGE_TOLERANCE);
    }

    #[tokio::test]
    async fn intents_quote_missing_quote_key_is_operation_failed() {
        let executor = NearExecutor::default();
        let capability = capability_id(NEAR_INTENTS_QUOTE_CAPABILITY_ID);
        let scope = scope();
        let input = intents_input();
        // An error envelope with no `quote` must surface a failure, not null fields.
        let egress = Arc::new(RecordingEgress::single(json!({ "error": "no route" })));

        let error = executor
            .dispatch(request(&capability, &scope, &input, Some(egress)))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::OperationFailed);
    }
}
