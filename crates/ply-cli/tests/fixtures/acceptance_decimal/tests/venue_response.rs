use acceptance_decimal::{DomainRecord, MapError, map_response};
use serde::Deserialize;

#[derive(Deserialize)]
struct Expected {
    records: Vec<DomainRecord>,
}

#[test]
fn decimal_strings_map_to_expected_records() {
    let raw = std::fs::read("tests/fixtures/venue-response.json").unwrap();
    let expected: Expected = serde_json::from_slice(
        &std::fs::read("tests/fixtures/venue-response.expected.json").unwrap(),
    )
    .unwrap();
    let actual = map_response(&raw).expect("the representative success response must map");
    assert!(
        !actual.is_empty(),
        "a success response containing valid records must not become zero records"
    );
    assert_eq!(actual, expected.records);
}

#[test]
fn empty_malformed_upstream_and_partial_failure_are_distinct() {
    assert_eq!(
        map_response(br#"{"status":"ok","records":[]}"#),
        Ok(Vec::new())
    );
    assert_eq!(map_response(b"not json"), Err(MapError::MalformedEnvelope));
    assert_eq!(
        map_response(br#"{"status":"error","error":"temporarily unavailable"}"#),
        Err(MapError::Upstream("temporarily unavailable".into()))
    );
    assert!(matches!(
        map_response(
            br#"{"status":"ok","records":[{"instrument":"BTC-HKD","price":"12.5000","quantity":"2"},{"instrument":"ETH-HKD","price":"broken","quantity":"4"}]}"#
        ),
        Err(MapError::InvalidRecord { index: 1, .. })
    ));
}
