use std::{future::Future, pin::pin};

use actix_http::{
    body::{BodySize, MessageBody},
    header::{HeaderMap, HeaderName, HeaderValue},
    Method, Payload, RequestHeadType, ResponseHead, StatusCode, Version,
};
use actix_utils::future::poll_fn;
use bytes::Bytes;
use h2::{
    client::{Builder, Connection, SendRequest},
    SendStream,
};
use http::{
    header::{CONNECTION, CONTENT_LENGTH, HOST, TRANSFER_ENCODING},
};
use log::trace;

use super::{
    config::ConnectorConfig,
    connection::{ConnectionIo, H2Connection},
    error::SendRequestError,
};
use crate::BoxError;

pub(crate) async fn send_request<Io, B>(
    mut io: H2Connection<Io>,
    head: RequestHeadType,
    body: B,
) -> Result<(ResponseHead, Payload), SendRequestError>
where
    Io: ConnectionIo,
    B: MessageBody,
    B::Error: Into<BoxError>,
{
    trace!("Sending client request: {:?} {:?}", head, body.size());

    let head_req = head.as_ref().method == Method::HEAD;
    let length = body.size();
    let eof = matches!(length, BodySize::None | BodySize::Sized(0));

    // h2 0.4 uses http 1 while the Actix request/response model remains on http 0.2.
    // This client-side conversion is paired with actix-http's server dispatcher boundary.
    let mut req = http_1::Request::new(());
    *req.uri_mut() = head
        .as_ref()
        .uri
        .to_string()
        .parse::<http_1::Uri>()
        .map_err(h2_boundary_error)?;
    *req.method_mut() = http_1::Method::from_bytes(head.as_ref().method.as_str().as_bytes())
        .map_err(h2_boundary_error)?;
    *req.version_mut() = http_1::Version::HTTP_2;

    let mut skip_len = true;
    // let mut has_date = false;

    // Content length
    let _ = match length {
        BodySize::None => None,

        BodySize::Sized(0) => {
            req.headers_mut().insert(
                http_1::header::CONTENT_LENGTH,
                http_1::HeaderValue::from_static("0"),
            )
        }

        BodySize::Sized(len) => {
            let mut buf = itoa::Buffer::new();

            req.headers_mut().insert(
                http_1::header::CONTENT_LENGTH,
                http_1::HeaderValue::from_str(buf.format(len)).map_err(h2_boundary_error)?,
            )
        }

        BodySize::Stream => {
            skip_len = false;
            None
        }
    };

    // Extracting extra headers from RequestHeadType. HeaderMap::new() does not allocate.
    let (head, extra_headers) = match head {
        RequestHeadType::Owned(head) => (RequestHeadType::Owned(head), HeaderMap::new()),
        RequestHeadType::Rc(head, extra_headers) => (
            RequestHeadType::Rc(head, None),
            extra_headers.unwrap_or_else(HeaderMap::new),
        ),
    };

    // merging headers from head and extra headers.
    let headers = head
        .as_ref()
        .headers
        .iter()
        .filter(|(name, _)| !extra_headers.contains_key(*name))
        .chain(extra_headers.iter());

    // copy headers
    for (key, value) in headers {
        match *key {
            // TODO: consider skipping other headers according to:
            //       https://datatracker.ietf.org/doc/html/rfc7540#section-8.1.2.2
            // omit HTTP/1.x only headers
            CONNECTION | TRANSFER_ENCODING | HOST => continue,
            CONTENT_LENGTH if skip_len => continue,
            // DATE => has_date = true,
            _ => {}
        }
        let key = http_1::header::HeaderName::from_bytes(key.as_str().as_bytes())
            .map_err(h2_boundary_error)?;
        let sensitive = value.is_sensitive();
        let mut value = http_1::HeaderValue::from_bytes(value.as_bytes()).map_err(h2_boundary_error)?;
        value.set_sensitive(sensitive);
        req.headers_mut().append(key, value);
    }

    let res = poll_fn(|cx| io.poll_ready(cx)).await;
    if let Err(err) = res {
        io.on_release(err.is_io() || err.is_go_away());
        return Err(SendRequestError::from(err));
    }

    let resp = match io.send_request(req, eof) {
        Ok((fut, send)) => {
            io.on_release(false);

            if !eof {
                send_body(body, send).await?;
            }
            fut.await.map_err(SendRequestError::from)?
        }
        Err(err) => {
            io.on_release(err.is_io() || err.is_go_away());
            return Err(err.into());
        }
    };

    let (parts, body) = resp.into_parts();
    let payload = if head_req { Payload::None } else { body.into() };

    let status = StatusCode::from_u16(parts.status.as_u16()).map_err(h2_boundary_error)?;
    let version = match parts.version {
        http_1::Version::HTTP_09 => Version::HTTP_09,
        http_1::Version::HTTP_10 => Version::HTTP_10,
        http_1::Version::HTTP_11 => Version::HTTP_11,
        http_1::Version::HTTP_2 => Version::HTTP_2,
        http_1::Version::HTTP_3 => Version::HTTP_3,
        _ => return Err(SendRequestError::Custom(Box::new(H2BoundaryError), Box::new("unsupported h2 HTTP version"))),
    };
    let mut headers = HeaderMap::with_capacity(parts.headers.len());
    for (name, value) in &parts.headers {
        let name = HeaderName::from_bytes(name.as_str().as_bytes()).map_err(h2_boundary_error)?;
        let sensitive = value.is_sensitive();
        let mut value = HeaderValue::from_bytes(value.as_bytes()).map_err(h2_boundary_error)?;
        value.set_sensitive(sensitive);
        headers.append(name, value);
    }

    let mut head = ResponseHead::new(status);
    head.version = version;
    head.headers = headers;
    Ok((head, payload))
}

#[derive(Debug)]
struct H2BoundaryError;

impl std::fmt::Display for H2BoundaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HTTP metadata could not cross the h2/http type boundary")
    }
}

impl std::error::Error for H2BoundaryError {}

fn h2_boundary_error<E>(_err: E) -> SendRequestError
where
    E: std::error::Error + 'static,
{
    SendRequestError::Custom(Box::new(H2BoundaryError), Box::new("invalid h2 HTTP metadata"))
}

async fn send_body<B>(body: B, mut send: SendStream<Bytes>) -> Result<(), SendRequestError>
where
    B: MessageBody,
    B::Error: Into<BoxError>,
{
    let mut buf = None;

    let mut body = pin!(body);

    loop {
        if buf.is_none() {
            match poll_fn(|cx| body.as_mut().poll_next(cx)).await {
                Some(Ok(b)) => {
                    send.reserve_capacity(b.len());
                    buf = Some(b);
                }
                Some(Err(err)) => return Err(SendRequestError::Body(err.into())),
                None => {
                    if let Err(err) = send.send_data(Bytes::new(), true) {
                        return Err(err.into());
                    }
                    send.reserve_capacity(0);
                    return Ok(());
                }
            }
        }

        match poll_fn(|cx| send.poll_capacity(cx)).await {
            None => return Ok(()),
            Some(Ok(cap)) => {
                let b = buf.as_mut().unwrap();
                let len = b.len();
                let bytes = b.split_to(std::cmp::min(cap, len));

                if let Err(err) = send.send_data(bytes, false) {
                    return Err(err.into());
                }
                if !b.is_empty() {
                    send.reserve_capacity(b.len());
                } else {
                    buf = None;
                }
                continue;
            }
            Some(Err(err)) => return Err(err.into()),
        }
    }
}

pub(crate) fn handshake<Io: ConnectionIo>(
    io: Io,
    config: &ConnectorConfig,
) -> impl Future<Output = Result<(SendRequest<Bytes>, Connection<Io, Bytes>), h2::Error>> {
    let mut builder = Builder::new();
    builder
        .initial_window_size(config.stream_window_size)
        .initial_connection_window_size(config.conn_window_size)
        .enable_push(false);
    builder.handshake(io)
}
