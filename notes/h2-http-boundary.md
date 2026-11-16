# Shared HTTP metadata

The Actix server, router, client, and HTTP/2 transport use `http` 1.x metadata.
Applications can share these types with framework-neutral HTTP handlers without
serializing and reparsing methods, URIs, status codes, header names, or values.

## Coupled packages

| Package | Shared contract |
| --- | --- |
| `actix-http` | Exposes `http` 1.x metadata through its request and response heads and uses the same types as `h2` 0.4. |
| `actix-router` | Accepts the same `http::Uri` in `Url` and `ResourcePath`. |
| `awc` | Uses the same HTTP metadata and concrete `h2` stream types as `actix-http`. |
| `actix-http-test` | Uses the same method types as the paired client and server. |

Consumers that name `http` types directly must use version 1.x. An application
patching this fork must select compatible `actix-http` and `actix-router`
packages together. Applications using `awc` or the test server must also select
the paired client packages. Actix Web obtains its HTTP metadata types from
`actix-http` and its URI router from `actix-router`.

## Ownership and representation

HTTP/2 ingress moves the accepted request's method, URI, version, and header
values into the Actix request head. Client ingress similarly moves response
metadata. Outbound request URIs and borrowed header values use same-type clones,
which share their existing byte storage. These transfers preserve raw URI
spelling, including escapes, authority, and query data; they do not normalize or
reparse the URI.

Actix's `HeaderMap` and `http::HeaderMap` have different collection layouts.
Transferring between them still allocates destination collection storage and
iterates the headers. Owned header values move between collections; borrowed
values share their storage through `HeaderValue::clone`. No per-value byte copy
or cross-version owner wrapper is needed. Repeated values retain their order
and each value retains its `is_sensitive` flag, including custom header names
and opaque value bytes. The flag remains available for local Debug redaction
and the HTTP/2 encoder's indexing policy; receiver-side flags are governed by
the HTTP/2 decoder.

Bodies retain the existing streaming `Payload` and `Bytes` ownership. Metadata
transfer does not collect or copy body data. HTTP/2 body framing, HEAD handling,
hop-by-hop header filtering, flow control, and graceful shutdown remain owned
by the transport.

## Parsing and integration

HTTP/1 request lines and headers are parsed by `httparse` and validated using
the shared `http` types. The accepted metadata follows `http` 1.x parsing rules,
including valid extension-method tokens and UTF-8 paths and queries. URI
control characters and invalid UTF-8, and malformed header names, are rejected.
Router path quoting and parameter extraction remain separate from the raw URI.

Integration tests cover actual HTTP/1 parsing and framing, raw URI preservation,
repeated and opaque headers, sensitivity flags and shared storage, plus HTTP/2
negotiation, flow control, and shutdown. A typed request conversion test alone
does not exercise the parser boundary.

## Upstream retirement

Remove the fork overrides when released upstream packages provide a compatible
shared HTTP 1.x metadata graph across `actix-http`, `actix-router`, and any
selected client packages, together with the maintained HTTP/2 and transport
security behavior. Update the coupled packages as one dependency change and
validate the application's ingress, streaming, TLS, and replay contracts.
