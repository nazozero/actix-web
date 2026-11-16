use actix_http::header::{HeaderMap, HeaderName, HeaderValue};

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
    let mut h2 = http::HeaderMap::new();
    for (name, value) in &original {
        let shared = value.clone();
        assert_eq!(shared.as_bytes(), value.as_bytes());
        assert_eq!(shared.as_bytes().as_ptr(), value.as_bytes().as_ptr());
        assert_eq!(shared.is_sensitive(), value.is_sensitive());
        if value.is_sensitive() {
            assert!(!format!("{:?}", shared).contains("first"));
        }
        h2.append(name.clone(), shared);
    }
    let round_trip = HeaderMap::from(h2);
    for (source, shared) in original
        .get_all("x-private-context")
        .zip(round_trip.get_all("x-private-context"))
    {
        assert_eq!(shared.as_bytes(), source.as_bytes());
        assert_eq!(shared.as_bytes().as_ptr(), source.as_bytes().as_ptr());
        assert_eq!(shared.is_sensitive(), source.is_sensitive());
        if source.is_sensitive() {
            assert!(!format!("{:?}", shared).contains("first"));
        }
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
