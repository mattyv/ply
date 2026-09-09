use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainRecord {
    pub instrument: String,
    /// Price in the provider's documented four-decimal fixed-point scale.
    pub price_ticks: i64,
    pub quantity: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapError {
    MalformedEnvelope,
    Upstream(String),
    InvalidRecord { index: usize, reason: String },
}

#[derive(Deserialize)]
struct WireEnvelope {
    status: String,
    #[serde(default)]
    records: Vec<WireRecord>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct WireRecord {
    instrument: String,
    price: String,
    quantity: String,
}

/// Production entry point: raw provider bytes enter here. One invalid record
/// rejects the batch with its index; it is never silently dropped.
pub fn map_response(raw: &[u8]) -> Result<Vec<DomainRecord>, MapError> {
    let envelope: WireEnvelope =
        serde_json::from_slice(raw).map_err(|_| MapError::MalformedEnvelope)?;
    if envelope.status != "ok" {
        return Err(MapError::Upstream(
            envelope
                .error
                .unwrap_or_else(|| "upstream returned an error without a message".into()),
        ));
    }

    envelope
        .records
        .into_iter()
        .enumerate()
        .map(|(index, record)| {
            let price_ticks = parse_price(&record.price).ok_or_else(|| MapError::InvalidRecord {
                index,
                reason: format!("price {:?} is not a four-decimal string", record.price),
            })?;
            let quantity =
                record
                    .quantity
                    .parse::<u64>()
                    .map_err(|_| MapError::InvalidRecord {
                        index,
                        reason: format!("quantity {:?} is not an unsigned integer", record.quantity),
                    })?;
            Ok(DomainRecord {
                instrument: record.instrument,
                price_ticks,
                quantity,
            })
        })
        .collect()
}

fn parse_price(text: &str) -> Option<i64> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 4
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let whole = whole.parse::<i64>().ok()?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<i64>().ok()? * 10_i64.pow((4 - fraction.len()) as u32)
    };
    whole.checked_mul(10_000)?.checked_add(fraction)
}
