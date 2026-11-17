//! The Z.AI quota endpoint: its URL, the body's shape, and how a body or
//! a failure updates the snapshot.

use serde_json::Value;

/// The usage endpoint the OpenCode `zai-coding-plan` provider talks to.
pub const DEFAULT_API_URL: &str = "https://api.z.ai/api/monitor/usage/quota/limit";

/// A successful sample as parsed from the endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// Used quota in percent (the API counts usage, not what is left).
    pub percentage: f64,
    /// When the quota resets (ms epoch).
    pub next_reset_ms: Option<i64>,
    /// The plan tier (`data.level`).
    pub level: Option<String>,
}

/// Parses the endpoint's body: `.data.limits[]` where `type ==
/// "TOKENS_LIMIT"` carries `percentage` (used %) and `nextResetTime` (ms
/// epoch); `data.level` is the plan tier. `{"code": N, "msg": "..."}` is
/// the API's error shape.
pub fn parse_quota(body: &Value) -> Result<Sample, String> {
    let data = match body.get("data") {
        Some(d) if !d.is_null() => d,
        _ => return Err(api_error(body)),
    };
    let limits = data
        .get("limits")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "malformed quota response: no data.limits".to_string())?;
    let tokens = limits
        .iter()
        .find(|l| l.get("type").and_then(|t| t.as_str()) == Some("TOKENS_LIMIT"))
        .ok_or_else(|| "no TOKENS_LIMIT entry in quota response".to_string())?;
    let percentage = tokens
        .get("percentage")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "TOKENS_LIMIT has no percentage".to_string())?;
    let next_reset_ms = tokens.get("nextResetTime").and_then(|v| v.as_i64());
    let level = data
        .get("level")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    Ok(Sample {
        percentage,
        next_reset_ms,
        level,
    })
}

/// Renders the API's error shape (`{code, msg}`) as one line.
fn api_error(body: &Value) -> String {
    let code = body.get("code").and_then(|v| v.as_i64());
    let msg = body
        .get("msg")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown API error");
    match code {
        Some(c) => format!("API error: {msg} (code {c})"),
        None => msg.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body() -> Value {
        json!({
            "success": true,
            "data": {
                "level": "lite",
                "limits": [
                    {"type": "PROMPT_LIMIT", "percentage": 3.0},
                    {"type": "TOKENS_LIMIT",
                     "percentage": 41.7,
                     "nextResetTime": 4102444800000_i64,
                     "currentCount": 417, "currentLimit": 1000}
                ]
            }
        })
    }

    #[test]
    fn parses_tokens_limit() {
        let s = parse_quota(&body()).unwrap();
        assert_eq!(s.percentage, 41.7);
        assert_eq!(s.next_reset_ms, Some(4_102_444_800_000));
        assert_eq!(s.level.as_deref(), Some("lite"));
    }

    #[test]
    fn parses_without_optional_fields() {
        let s =
            parse_quota(&json!({"data": {"limits": [{"type": "TOKENS_LIMIT", "percentage": 0}]}}))
                .unwrap();
        assert_eq!(s.percentage, 0.0);
        assert_eq!(s.next_reset_ms, None);
        assert_eq!(s.level, None);
    }

    #[test]
    fn api_error_shape() {
        let err = parse_quota(&json!({"code": 1002, "msg": "request too fast"})).unwrap_err();
        assert_eq!(err, "API error: request too fast (code 1002)");
    }

    #[test]
    fn null_data_is_api_error() {
        let err = parse_quota(&json!({"success": true, "data": null})).unwrap_err();
        assert_eq!(err, "unknown API error");
    }

    #[test]
    fn missing_tokens_limit() {
        let err = parse_quota(&json!({"data": {"limits": []}})).unwrap_err();
        assert_eq!(err, "no TOKENS_LIMIT entry in quota response");
    }
}
