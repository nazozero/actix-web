use super::*;

#[actix_rt::test]
async fn h2_header_conversions_preserve_each_values_sensitivity_and_bytes() {
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
    let mut head = ResponseHead::new(http::StatusCode::OK);
    head.headers = original;
    let response = prepare_response(ServiceConfig::default(), &head, &mut BodySize::Sized(0));
    let h2 = response.headers();
    for (source, shared) in head
        .headers
        .get_all("x-private-context")
        .zip(h2.get_all("x-private-context"))
    {
        assert_eq!(shared.as_bytes(), source.as_bytes());
        assert_eq!(shared.as_bytes().as_ptr(), source.as_bytes().as_ptr());
        assert_eq!(shared.is_sensitive(), source.is_sensitive());
        if source.is_sensitive() {
            assert!(!format!("{:?}", shared).contains("first"));
        }
    }
    let round_trip = HeaderMap::from(response.into_parts().0.headers);
    for (source, shared) in head
        .headers
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
