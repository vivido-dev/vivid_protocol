# Vivid Protocol 1.5 Web Bindings

This file is a normative part of the
[Vivid Protocol 1.5 specification](vivid-protocol-1.5-spec.md) and defines
`web-carrier-v1`.

## 1. Scope and standards basis

The binding is based on the 6 July 2026
[W3C WebTransport Working Draft](https://www.w3.org/TR/webtransport/), the July 2026
[WebTransport over HTTP/3 Internet-Draft](https://datatracker.ietf.org/doc/draft-ietf-webtrans-http3/),
and the [WHATWG WebSockets Standard](https://websockets.spec.whatwg.org/). These upstream
specifications remain subject to change. A deployment MUST version-gate the exact browser and
server behavior it implements and retain WebSocket fallback.

`web-carrier-v1` changes transport mapping and binding admission. It does not create a gateway
authentication bypass, alter the Vivid preface, add a route header, or make browser Origin into
Vivid authority.

## 2. Browser-session admission

WebTransport fetch credentials mode is `omit`, so ambient cookies are not assumed to authenticate
the WebTransport request. Before connection:

1. the browser authenticates to a same-origin HTTPS application endpoint under the application's
   normal policy;
2. that endpoint validates user, controller lease, Origin, and browser generation;
3. it creates a random 256-bit, one-use web-session admission ticket;
4. it binds the ticket to exact Origin, browser generation, route, allowed carrier mode, stream
   count, byte budget, and an activation timeout no greater than 30 seconds; and
5. the browser supplies the ticket in the WebTransport `headers` option, never the URL.

The request header is:

```text
Vivid-Admission: <unpadded-base64url-32-bytes>
```

If the deployed browser/server combination cannot set and validate this header, it uses the
WebSocket fallback. A query parameter, path component, fragment, cookie workaround, or loggable
redirect is forbidden.

The server validates the Fetch-supplied Origin; it does not trust an Origin string echoed by a
native client. It atomically consumes the admission after validating the WebTransport request and
before accepting Vivid streams. A downstream Vivid authentication failure does not make it
reusable.

This admission ticket authorizes only one bounded web carrier. It is not:

- `VIVID_ROOT_SECRET`;
- a Vivid context or session lease;
- an activation or resume secret;
- a channel authentication key; or
- accepted by a native Vivid listener.

The Vivid session inside the carrier performs normal root, lease, or resume authentication.

## 3. Gateway trust modes

A web gateway chooses exactly one mode per route.

### 3.1 Transparent byte route

A transparent gateway:

- authenticates and bounds only the web carrier;
- forwards each Vivid connection byte-for-byte;
- cannot rewrite `HELLO`, `WELCOME`, a proof, profile, object ID, or channel tag;
- exposes no underlying local endpoint to the browser; and
- routes only streams bound to the admitted browser generation and controller route.

End-to-end Vivid authority terminates at the browser or remote native endpoint, not the gateway.

### 3.2 Terminating Vivid gateway

A terminating gateway:

- authenticates inbound Vivid normally;
- enforces a complete inbound resource contract;
- originates an independent outbound Vivid session;
- translates surfaces, tracks, nodes, channels, revisions, and flow under complete owner tuples;
- uses independent secrets and keys on each hop; and
- intersects profile, capture, resource, and input policy.

It never substitutes a zero token, root-token-shaped placeholder, or inbound secret in an outbound
`HELLO`.

The route's mode is fixed before web-session admission. A deployment error cannot turn a
terminating-only credential into transparent root authority.

## 4. WebTransport binding

### 4.1 Construction and mode

The browser requests:

```text
protocols: ["vivid-1.5"]
allowPooling: false
headers: {
  "Vivid-Admission": admission,
  "Vivid-Channel-Binding": "exporter-v1" | "none"
}
```

It verifies after `ready`:

- the selected protocol is exactly `vivid-1.5`;
- the server-confirmed binding mode equals the request; and
- the actual `reliability` mode.

Two carrier modes exist:

| Mode | Required construction | Property |
|---|---|---|
| `independent-streams` | `requireUnreliable: true`; verify `reliability == "supports-unreliable"` | HTTP/3/UDP required; separate streams have independent retransmission ordering, subject to shared congestion and connection flow |
| `reliable-only-degraded` | `requireUnreliable: false`; observed `reliability == "reliable-only"` | Explicit HTTP/2/TCP degraded mode; no claim of independent loss recovery |

An acceptance test requiring lane loss isolation uses `independent-streams`. It MUST NOT silently
accept reliable-only mode. A product MAY explicitly allow degraded mode for view-only or measured
deployments.

`allowPooling` is false even though that is the current API default. A dedicated connection avoids
unreviewed cross-session pooling and makes exporter and congestion scope explicit.

### 4.2 Stream mapping

Each reliable bidirectional WebTransport stream maps to exactly one Vivid connection:

- the endpoint acting as Vivid initiator creates the stream;
- byte zero is byte zero of the 16-byte Vivid 1.5 preface;
- control, lane, and track roles are identified by the preface and authenticated first record;
- no `VVWT`, route, lane, stream-ID, or source header precedes the preface;
- WebTransport write/read boundaries are irrelevant; Vivid record framing controls;
- unidirectional streams and datagrams are not Vivid transports; and
- unexpected or excess streams are reset before Vivid allocation.

The web admission establishes route capacity before application streams are accepted. Early
streams are reset, not buffered.

Resetting a track stream detaches only that track channel. Resetting the interactive stream revokes
input. Resetting control or closing the WebTransport session triggers the clean/unclean session
lease rules.

### 4.3 Exporter channel binding

Direct WebTransport endpoints SHOULD select `exporter-v1` when both Vivid endpoints can access the
same session exporter. After the WebTransport session is ready, each obtains:

```text
carrier_binding_key = exportKeyingMaterial(
    label   = UTF8("EXPORTER-VIVID-1.5"),
    context = UTF8(selected_protocol) || SHA-256(admission_ticket),
    length  = 32
)
```

The ticket hash is used only inside the exporter context and is never logged. The value becomes
the `carrier_binding_key` in the Vivid handshake KDF. It is not transmitted.

The HTTP response confirms:

```text
Vivid-Channel-Binding: exporter-v1
```

A transparent gateway relaying a Vivid peer on another transport selects `none`, because the
remote peer cannot access this WebTransport exporter. Both Vivid peers then use the all-zero
binding key and rely on end-to-end Vivid authentication plus web-session admission. A gateway MUST
NOT pretend to exporter-bind an end-to-end proof it cannot recompute.

### 4.4 Bounds

Before accepting application data, the server bounds:

- unauthenticated and admitted WebTransport sessions;
- admission tickets and their activation state;
- bidirectional and unexpected unidirectional streams;
- preface/first-record buffers;
- control, interactive, realtime, and bulk stream counts;
- per-stream and aggregate bytes;
- outstanding writes and read tasks; and
- browser-generation route lifetime.

The binding uses the lower of native Vivid limits and web deployment limits. It resets a stream or
closes the session when a bound is exceeded; it does not accumulate early or excess data.

Independent streams still share QUIC congestion and may share connection-level flow control. The
binding MUST NOT state that one stream is performance-independent of the entire WebTransport
connection.

## 5. WebSocket fallback

Classic browser WebSocket exposes sender-side `bufferedAmount`, but no receiver-controlled byte
stream backpressure equivalent to WebTransport streams. The fallback therefore has stricter finite
limits.

### 5.1 Connection mapping and subprotocols

One WebSocket maps to one Vivid connection. The browser and bridge select exactly one subprotocol:

| Vivid connection | WebSocket subprotocol |
|---|---|
| Control | `vivid-1.5-control` |
| Interactive lane | `vivid-1.5-interactive` |
| Realtime track | `vivid-1.5-realtime` |
| Bulk track | `vivid-1.5-bulk` |

The selected value must equal the preface/opened lane. A mismatch closes before allocation.

After binding admission, binary messages carry consecutive Vivid stream bytes. Message boundaries
have no Vivid meaning. Text messages are forbidden. Per-message compression and every WebSocket
compression extension are forbidden.

Before those stream bytes, the first client binary message contains exactly the raw 32-byte
one-use socket admission value minted by a same-origin authenticated HTTPS request. The gateway
consumes that message at the binding layer and does not forward it. After admission, the first
binding payload byte in the Vivid-initiator-to-peer direction is Vivid preface byte zero. No Vivid
bytes are accepted or buffered before admission succeeds.

Separate WebSockets are used for control, interactive, and each realtime/bulk track connection.
This preserves connection-level failure and TCP-queue isolation at the cost of a bounded socket
count. A deployment that routes all of them through one upstream TCP or proxy queue reports that
degradation.

### 5.2 Mandatory web ceilings

Unless a deployment advertises lower values:

| Bound | Maximum |
|---|---:|
| WebSocket binary message chunk | 65,536 bytes |
| Control Vivid record body | 262,144 bytes |
| Interactive record body | 65,536 bytes |
| Media Vivid record body | 8,388,608 bytes |
| Per-connection incomplete-record reassembly | One accepted record plus 24 bytes |
| Aggregate browser reassembly and queued decoded input | 33,554,432 bytes |
| Pending browser event chunks per connection | 128 |

The browser uses `binaryType = "arraybuffer"`. A sender splits the Vivid byte stream into messages
no larger than 64 KiB. The receiver parses incrementally and never concatenates an unbounded list
of messages.

The negotiated Vivid record ceiling and absolute media flow windows fit inside the aggregate web
budget after accounting for copies and decoder input. A browser presenter does not grant a channel
window that could force the bridge/browser pipeline above this budget.

If the event loop or receive pump stalls such that a per-channel or aggregate bound would be
exceeded, the bridge closes the affected track WebSocket, or the whole carrier for control
overflow. It does not buffer beyond the bound. Track closure invokes ordinary channel-generation
recovery.

The browser sender stops calling `send()` at its configured `bufferedAmount` high watermark and
resumes below a low watermark. Its application queue plus `bufferedAmount` remains within the
same finite aggregate budget.

### 5.3 WebSocket admission

The server:

- requires `wss:` across a host boundary;
- validates the Fetch-supplied Origin;
- binds every socket to one authenticated browser generation and route;
- consumes the exact first-message socket admission described above, bound to the selected
  subprotocol/lane and expiring within 30 seconds;
- never places admission, Vivid authority, or channel authentication in the URL; and
- bounds sockets before relaying a Vivid preface.

The binding admission is not substituted into Vivid `HELLO`.

## 6. Browser input requirements

The interactive connection is serviced independently of canvas/media decode and bulk socket
writes. The browser:

- emits physical key transitions without synthetic repeat;
- carries the complete input tuple on every event;
- does not allow a stalled render loop to delay watchdog renewal beyond its bound;
- revokes input on focus/visibility/permission loss under the desktop input state machine; and
- never restores a prior grant automatically.

A bulk parser, decoder, or WebSocket message handler cannot own the interactive writer or block
`INPUT_REVOKED`, `INPUT_RESET`, or renewal.

## 7. Logging and privacy

Admission tickets, root/activation/resume secrets, Vivid proofs, channel tags, exporter values, and
input contents never appear in:

- URLs;
- access logs;
- JavaScript console output;
- analytics;
- traces;
- close reasons;
- service-worker state;
- browser persistent storage; or
- combined correlation identifiers.

WebTransport stream IDs and WebSocket route IDs are not combined with session/track IDs in public
diagnostics. Sanitized counters may be reported without secret or content values.

## 8. Web conformance

The same Vivid session, surface, media, input, flow, recovery, and two-owner isolation tests run
through WebTransport and WebSocket.

Binding-specific tests:

- reject wrong Origin, generation, route, expired/reused admission, and excessive streams/sockets;
- verify selected WebTransport protocol and reliability mode;
- require HTTP/3 mode when independent-stream acceptance is claimed;
- verify exporter agreement and reject binding-mode mismatch;
- prove byte zero on every stream/socket is Vivid preface byte zero;
- fuzz every split/coalesced preface, record header, and body boundary;
- reject datagrams, unidirectional streams, early streams, text WebSocket messages, compression,
  and lane/subprotocol mismatch;
- stall the browser event loop and prove every memory bound closes rather than grows;
- reset video while audio/control/input remain live;
- reset interactive and prove input releases while media remains live;
- reset control and verify suspend-or-cleanup policy; and
- saturate bulk traffic while `PING`, input revoke/reset, realtime flow, and explicit authority
  revocation continue.
