use super::*;

#[test]
fn h2_header_conversions_preserve_each_values_sensitivity_and_bytes() {
    let mut original = HeaderMap::new();
    for (bytes, sensitive) in [
        (b"first".as_slice(), true),
        (b"second".as_slice(), false),
        (b"\x80".as_slice(), true),
    ] {
        let mut value = HeaderValue::from_bytes(bytes).unwrap();
        value.set_sensitive(sensitive);
        original.append(HeaderName::from_static("x-private-context"), value);
    }
    let mut h2 = http_1::HeaderMap::new();
    for (name, value) in &original {
        let converted = header_value_to_h2(value).ok().expect("valid h2 header value");
        assert_eq!(converted.as_bytes(), value.as_bytes());
        assert_eq!(converted.is_sensitive(), value.is_sensitive());
        if value.is_sensitive() {
            assert!(!format!("{:?}", converted).contains("first"));
        }
        h2.append(
            http_1::header::HeaderName::from_bytes(name.as_str().as_bytes()).unwrap(),
            converted,
        );
    }
    let mut round_trip = HeaderMap::new();
    for (name, value) in &h2 {
        let converted = header_value_from_h2(value).unwrap();
        assert_eq!(converted.as_bytes(), value.as_bytes());
        assert_eq!(converted.is_sensitive(), value.is_sensitive());
        if value.is_sensitive() {
            assert!(!format!("{:?}", converted).contains("first"));
        }
        round_trip.append(
            HeaderName::from_bytes(name.as_str().as_bytes()).unwrap(),
            converted,
        );
    }
    let values = round_trip.get_all("x-private-context").collect::<Vec<_>>();
    assert_eq!(
        values.iter().map(|v| v.as_bytes()).collect::<Vec<_>>(),
        [
            b"first".as_slice(),
            b"second".as_slice(),
            b"\x80".as_slice()
        ]
    );
    assert_eq!(
        values.iter().map(|v| v.is_sensitive()).collect::<Vec<_>>(),
        [true, false, true]
    );
}
