# Maintained `actix-http` h2 0.4 / http 1 transport boundary

## Current maintenance base

The h2 transport patch is now based on upstream stable `actix-http` 3.18.12,
tag `http-v3.18.12`, commit `a1d3c932cad97eade59bae5640f9ff10ac6594ed`.
The original paired server/client patch applies without conflicts. Upstream
HTTP/1 Host validation, chunk framing and compression fixes stay in place;
the public Actix `http` 0.2 model and private h2 `http` 1 conversion remain.

Rust 1.99.0 compilation and H2/TLS regression results for this maintenance
revision are recorded separately. The counts and hashes below are historical
evidence for the original 3.13.3 port and do not validate this new base.

## Historical PoC source

- Upstream: `https://github.com/actix/actix-web`
- Exact `main` commit: `e5e99114d6e871d48dbe3dada7f634821cb3885d`
- `actix-http` package version at that commit: `3.13.3`

## Dependency and type boundary

| Boundary | Existing type | h2 0.4 type | Required PoC treatment |
|---|---|---|---|
| Actix public/internal HTTP model | `http` 0.2 (`crate::{Method, RequestHead, ResponseHead}`) | n/a | Keep unchanged: it is shared throughout Actix and NazoAuth’s dependency graph. |
| h2 server accepted request | h2 0.3 returns `http` 0.2 `Request<RecvStream>` | h2 0.4 returns `http` 1 `Request<RecvStream>` | Convert request method, URI, version and headers at the dispatcher ingress. Extensions are not consumed here. |
| h2 outbound response | `prepare_response` builds `http` 0.2 `Response<()>` | `SendResponse::send_response` requires `http` 1 `Response<()>` | Build the h2 response with `http` 1 and copy Actix response metadata across the boundary. |
| h2 client tests | `::http` 0.2 `Request<()>` | h2 0.4 client requires `http` 1 `Request<()>` | Use explicit `http_1` dependency in H2-only tests. |

`http` 0.2 and `http` 1 cannot share `Method`, `Uri`, `Version`, `HeaderName`,
`HeaderValue`, `HeaderMap`, request, or response types. The smallest safe port is
therefore dual-versioning `http`: retain `http` 0.2 for the Actix API and add an
aliased `http_1` only inside the h2 transport module and its direct tests.

## Conversion rules / failure behavior

1. H2 ingress converts method from its ASCII representation, URI from its
   serialized form, known HTTP versions by an exhaustive map, and every header
   name/value from bytes. An impossible conversion terminates the H2 dispatch
   with a protocol `DispatchError`; it is never silently dropped or rewritten.
2. H2 egress sets HTTP/2 explicitly, converts the status code numerically, and
   copies each header byte-for-byte into a new `http_1::HeaderMap`.
3. Hop-by-hop header removal and body framing remain in the existing
   `prepare_response` owner. The port must not disable HTTP/2 or weaken those
   existing semantics.

## Minimal compilation slice

`cargo check -p actix-http --features http2` is the first compiler gate. It
exercises the production h2 dispatcher without pulling TLS integration tests.
After it compiles, run Actix’s H2 test set serially. No NazoAuth checkout is
modified by this PoC.

## Observed gates

- `cargo check -p actix-http --features http2 --jobs 1`: **PASS**. The lockfile
  resolved `h2 0.4.17` and `http 1.5.0`, while retaining the old `h2 0.3.27`
  and `http 0.2.12` where other workspace packages still require them.
- `cargo test -p actix-http --features http2 --test test_h2_timer --jobs 1 --
  --test-threads=1`: **BLOCKED BEFORE TEST EXECUTION** (`exit 101`; see
  `test_h2_timer.log`). Cargo compiles direct dev-dependency `awc 3.8.2` before
  the test binary. `awc/src/client/h2proto.rs:130` receives an h2 0.3
  `RecvStream`, but `actix-http::Payload` now has a `From` implementation only
  for h2 0.4 `RecvStream`, so the package graph fails with E0277.

This is an API graph boundary, not an h2 test failure: `actix-http` publicly
exposes h2's concrete `RecvStream` through `Payload`. A safe upstream port
must either migrate the coupled `awc` client in the same compatible release, or
introduce an Actix-owned stream abstraction that avoids exposing a concrete h2
major version. Retaining parallel `From<RecvStream>` implementations is
impossible because the trait/type pair is identical apart from the dependency
version. A NazoAuth-only Cargo override would produce the same incompatibility
and is not a safe remediation.

The paired `actix-http` + `awc` migration in this branch closes that boundary.
Its recorded serial gates passed before publication:

- `cargo test -p actix-http --features http2 --jobs 1 -- --test-threads=1`:
  **PASS**, 308 passed, 0 failed, 2 ignored. Log SHA-256:
  `b09663b81310e33d27bf574d683170c01d76f775905d6dd0c89cab858aa4b67c`.
- `cargo test -p actix-http --features rustls-0_23,http2 --test test_rustls
  --jobs 1 -- --test-threads=1`: **PASS**, 21 passed, 0
  failed. Log SHA-256:
  `788bebf536443448f90042e314d3fd48c9f1e98cdf29527ea4c09f33ebb70cd7`.
- `cargo test -p awc --features rustls-0_23-webpki-roots --test
  test_rustls_client test_connection_reuse_h2 --jobs 1 -- --test-threads=1`:
  **PASS**, 1 passed,
  0 failed. Log SHA-256:
  `4c8138d25ebd0f2a2258f49104b23f5c3c182b1236ca86d21d1ce5a0c7c5d8b8`.

The evidence logs are intentionally not committed to this source branch.

## Static h2 0.3 consumer closure

The workspace manifests contain exactly two direct h2 0.3 consumers:

| Package | h2-facing production API / files | Required coupled change |
|---|---|---|
| `actix-http` | `src/h2/{mod,dispatcher}.rs`; `src/payload.rs` publishes `From<h2::RecvStream>`; `src/error.rs` publishes `h2::Error` variants | Upgrade h2, retain `http` 0.2 for Actix model, add private `http_1` boundary conversion. |
| `awc` | `src/client/{connection,h2proto,error}.rs`; `H2Connection` stores `SendRequest`; `h2proto` sends/receives concrete h2 request/response streams; public errors contain `h2::Error` | Upgrade h2 together with `actix-http`, add the paired HTTP metadata conversion at the client boundary. |

Direct h2 test consumers are `actix-http/tests/test_h2_timer.rs` and
`actix-http/tests/test_server.rs`; they must construct `http_1::Request` after
the library port. No other workspace manifest declares `h2`. `actix-web`,
`actix-router`, `actix-http-test`, and `actix-server` do not directly expose an
h2 type in this checkout. This closure excludes transitive h2 0.3 copies that
remain in unrelated dependency paths; they cannot satisfy the concrete public
types above and must not be used as an adapter.
