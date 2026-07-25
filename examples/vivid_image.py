#!/usr/bin/env python3
"""Display a PNG or JPEG in Vivido using Vivid Protocol directly.

This is intentionally self-contained: it uses only the Python standard library and implements the
small CBOR and wire-protocol subset needed for one retained encoded image.
"""

import argparse
import base64
import hashlib
import hmac
import math
import os
from pathlib import Path
import secrets
import socket
import struct
import sys
from dataclasses import dataclass
from typing import Any, Dict, Optional, Sequence, Set, Tuple


# Connection and record limits.
CONTROL_LIMIT = 1024 * 1024
MEDIA_LIMIT = 64 * 1024 * 1024

# Connection kinds.
CONTROL = 0
BLOB = 3

# Control records used by this demo.
HELLO = 0x0001
WELCOME = 0x0002
OK = 0x0003
ERROR = 0x0004
GOODBYE = 0x0007
DISPLAY_CHANGED = 0x0008
CREATE_IMAGE = 0x0102
SOURCE_READY = 0x0105
SOURCE_LOST = 0x0108
WAIT_SOURCE = 0x010F
WAIT_SATISFIED = 0x0110
BEGIN_TXN = 0x0200
CREATE_NODE = 0x0201
COMMIT_TXN = 0x0204
PRESENTED = 0x0206
ANCHOR_READY = 0x0207
CREDIT = 0x0400
ATTACH_CHANNEL = 0x8000
IMAGE_DATA = 0x8006

# Features required for inline placement plus encoded-image support.
FEATURE_RASTER_RGBA8 = 1
FEATURE_SCENE_TRANSACTIONS = 3
FEATURE_GRID_CELL_NODES = 4
FEATURE_CREDIT_FLOW_CONTROL = 5
FEATURE_ENCODED_IMAGE = 7
FEATURE_TEXT_ANCHORS = 13
FEATURE_OBSERVABILITY_CORE = 18

WAIT_FIRST_VISIBLE_PRESENTATION = 2

# Image and scene constants.
IMAGE_PNG = 1
IMAGE_JPEG = 2
COLOR_SPACE_SRGB = 1
RETENTION_DECODED_SOURCE = 1
COORDINATE_ANCHOR_CELL = 3
FIT_CONTAIN = 2
SAMPLING_LINEAR = 1
TEXT_LAYER_BETWEEN_BACKGROUND_AND_GLYPH = 1
BLEND_SOURCE_OVER = 0
PRESENT_NEXT_COMPOSITOR_FRAME = 0

ERROR_STALE_DISPLAY_GENERATION = 15


class VividError(RuntimeError):
    """A local protocol, input, or presenter error."""


class PresenterError(VividError):
    def __init__(self, code: int, request_id: int, diagnostic: str) -> None:
        super().__init__(f"presenter error {code}: {diagnostic}")
        self.code = code
        self.request_id = request_id


def _cbor_argument(major: int, value: int) -> bytes:
    if value < 0:
        raise VividError("CBOR argument cannot be negative")
    prefix = major << 5
    if value <= 23:
        return bytes((prefix | value,))
    if value <= 0xFF:
        return bytes((prefix | 24, value))
    if value <= 0xFFFF:
        return bytes((prefix | 25,)) + struct.pack(">H", value)
    if value <= 0xFFFFFFFF:
        return bytes((prefix | 26,)) + struct.pack(">I", value)
    if value <= 0xFFFFFFFFFFFFFFFF:
        return bytes((prefix | 27,)) + struct.pack(">Q", value)
    raise VividError("integer exceeds the Vivid CBOR range")


def cbor_encode(value: Any) -> bytes:
    """Encode the deterministic CBOR subset used by Vivid control messages."""

    if isinstance(value, bool):
        return b"\xf5" if value else b"\xf4"
    if value is None:
        return b"\xf6"
    if isinstance(value, int):
        if value >= 0:
            return _cbor_argument(0, value)
        return _cbor_argument(1, -1 - value)
    if isinstance(value, bytes):
        return _cbor_argument(2, len(value)) + value
    if isinstance(value, str):
        encoded = value.encode("utf-8")
        return _cbor_argument(3, len(encoded)) + encoded
    if isinstance(value, (list, tuple)):
        return _cbor_argument(4, len(value)) + b"".join(
            cbor_encode(item) for item in value
        )
    if isinstance(value, dict):
        if any(
            isinstance(key, bool) or not isinstance(key, int) or key < 0
            for key in value
        ):
            raise VividError("Vivid CBOR maps require unsigned integer keys")
        items = sorted(value.items())
        return _cbor_argument(5, len(items)) + b"".join(
            cbor_encode(key) + cbor_encode(item) for key, item in items
        )
    raise VividError(f"unsupported CBOR value: {type(value).__name__}")


class _CborDecoder:
    def __init__(self, data: bytes) -> None:
        self.data = data
        self.offset = 0

    def _take(self, length: int) -> bytes:
        end = self.offset + length
        if end > len(self.data):
            raise VividError("truncated CBOR value")
        value = self.data[self.offset : end]
        self.offset = end
        return value

    def _argument(self, additional: int) -> int:
        if additional <= 23:
            return additional
        widths = {24: 1, 25: 2, 26: 4, 27: 8}
        width = widths.get(additional)
        if width is None:
            raise VividError("unsupported CBOR argument")
        return int.from_bytes(self._take(width), "big")

    def value(self, depth: int = 0) -> Any:
        if depth > 16:
            raise VividError("CBOR nesting exceeds the Vivid limit")
        initial = self._take(1)[0]
        major = initial >> 5
        additional = initial & 0x1F
        if major == 0:
            return self._argument(additional)
        if major == 1:
            return -1 - self._argument(additional)
        if major in (2, 3):
            length = self._argument(additional)
            if length > 16 * 1024 * 1024:
                raise VividError("CBOR string exceeds the Vivid limit")
            raw = self._take(length)
            if major == 2:
                return raw
            try:
                return raw.decode("utf-8")
            except UnicodeDecodeError as error:
                raise VividError("CBOR text is not UTF-8") from error
        if major == 4:
            length = self._argument(additional)
            if length > 4096:
                raise VividError("CBOR array exceeds the Vivid limit")
            return [self.value(depth + 1) for _ in range(length)]
        if major == 5:
            length = self._argument(additional)
            if length > 4096:
                raise VividError("CBOR map exceeds the Vivid limit")
            result: Dict[int, Any] = {}
            previous = -1
            for _ in range(length):
                key = self.value(depth + 1)
                if isinstance(key, bool) or not isinstance(key, int) or key <= previous:
                    raise VividError(
                        "CBOR map keys are not strictly ordered unsigned integers"
                    )
                previous = key
                result[key] = self.value(depth + 1)
            return result
        if major == 7 and additional in (20, 21, 22):
            return {20: False, 21: True, 22: None}[additional]
        raise VividError("unsupported CBOR value")


def cbor_decode(data: bytes) -> Any:
    decoder = _CborDecoder(data)
    value = decoder.value()
    if decoder.offset != len(data):
        raise VividError("trailing bytes after CBOR value")
    return value


def envelope(
    request_id: int,
    payload: Dict[int, Any],
    transaction_id: Optional[int] = None,
    expected_generation: Optional[int] = None,
) -> bytes:
    fields: Dict[int, Any] = {0: request_id}
    if transaction_id is not None:
        fields[1] = transaction_id
    if expected_generation is not None:
        fields[2] = expected_generation
    fields[3] = payload
    return cbor_encode(fields)


def decode_envelope(body: bytes) -> Tuple[int, Dict[int, Any]]:
    value = cbor_decode(body)
    if not isinstance(value, dict) or not isinstance(value.get(0), int):
        raise VividError("invalid Vivid control envelope")
    payload = value.get(3)
    if not isinstance(payload, dict):
        raise VividError("Vivid control envelope is missing its payload")
    return value[0], payload


@dataclass
class Record:
    record_type: int
    object_id: int
    body: bytes


def _connect(endpoint: str) -> socket.socket:
    if endpoint.startswith("tcp:"):
        address = endpoint[4:]
        if address.startswith("["):
            closing = address.find("]")
            if (
                closing < 0
                or closing + 1 >= len(address)
                or address[closing + 1] != ":"
            ):
                raise VividError("invalid bracketed TCP endpoint")
            host, port_text = address[1:closing], address[closing + 2 :]
        else:
            try:
                host, port_text = address.rsplit(":", 1)
            except ValueError as error:
                raise VividError("TCP endpoint must include host and port") from error
        try:
            stream = socket.create_connection((host, int(port_text)), timeout=30)
        except ValueError as error:
            raise VividError("TCP endpoint has an invalid port") from error
        stream.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        return stream

    path = endpoint[5:] if endpoint.startswith("unix:") else endpoint
    if not os.path.isabs(path):
        raise VividError("Vivid Unix endpoint must be an absolute path")
    if not hasattr(socket, "AF_UNIX"):
        raise VividError("this Python runtime does not support Unix-domain sockets")
    stream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    stream.settimeout(30)
    stream.connect(path)
    return stream


class Connection:
    def __init__(self, endpoint: str, kind: int, body_limit: int) -> None:
        self.stream = _connect(endpoint)
        self.send_sequence = 0
        self.receive_sequence = 0
        self.send_limit = body_limit
        self.receive_limit = body_limit
        preface = struct.pack(">4sBBBBII", b"VIVD", 1, 0, kind, 0, body_limit, 0)
        self.stream.sendall(preface)

    def close(self) -> None:
        self.stream.close()

    def send_record(self, record_type: int, object_id: int, body: bytes) -> None:
        if len(body) > self.send_limit or len(body) > MEDIA_LIMIT:
            raise VividError(f"record body is too large ({len(body)} bytes)")
        self.send_sequence += 1
        header = struct.pack(
            ">IHHQQ", len(body), record_type, 0, object_id, self.send_sequence
        )
        self.stream.sendall(header + body)

    def _read_exact(self, length: int) -> bytes:
        chunks = bytearray()
        while len(chunks) < length:
            chunk = self.stream.recv(length - len(chunks))
            if not chunk:
                raise VividError("presenter closed the connection")
            chunks.extend(chunk)
        return bytes(chunks)

    def read_record(self) -> Record:
        body_length, record_type, flags, object_id, sequence = struct.unpack(
            ">IHHQQ", self._read_exact(24)
        )
        if flags & ~1:
            raise VividError("presenter record has unknown flags")
        if body_length > self.receive_limit or body_length > MEDIA_LIMIT:
            raise VividError("presenter record exceeds the configured limit")
        expected = self.receive_sequence + 1
        if sequence != expected:
            raise VividError(
                f"presenter record sequence is {sequence}, expected {expected}"
            )
        self.receive_sequence = sequence
        return Record(record_type, object_id, self._read_exact(body_length))


@dataclass
class ImageInfo:
    encoding: int
    width: int
    height: int
    data: bytes


@dataclass
class SourceReady:
    source_id: int
    ticket: bytes
    byte_credits: int
    packet_credits: int
    max_media_body: int


def inspect_image(path: Path) -> ImageInfo:
    data = path.read_bytes()
    if data.startswith(b"\x89PNG\r\n\x1a\n"):
        if len(data) < 24 or data[12:16] != b"IHDR":
            raise VividError("PNG is missing its IHDR header")
        width, height = struct.unpack(">II", data[16:24])
        encoding = IMAGE_PNG
    elif data.startswith(b"\xff\xd8"):
        width, height = _jpeg_dimensions(data)
        encoding = IMAGE_JPEG
    else:
        raise VividError("the demo supports PNG and JPEG images only")
    if width == 0 or height == 0 or width > 8192 or height > 8192:
        raise VividError(f"unsupported image dimensions: {width}x{height}")
    if not data or len(data) > MEDIA_LIMIT:
        raise VividError("encoded image exceeds the Vivid media-record limit")
    return ImageInfo(encoding, width, height, data)


def _jpeg_dimensions(data: bytes) -> Tuple[int, int]:
    offset = 2
    start_of_frame = {
        0xC0,
        0xC1,
        0xC2,
        0xC3,
        0xC5,
        0xC6,
        0xC7,
        0xC9,
        0xCA,
        0xCB,
        0xCD,
        0xCE,
        0xCF,
    }
    while offset < len(data):
        while offset < len(data) and data[offset] == 0xFF:
            offset += 1
        if offset >= len(data):
            break
        marker = data[offset]
        offset += 1
        if marker == 0x00 or marker == 0xD8 or 0xD0 <= marker <= 0xD7:
            continue
        if marker in (0xD9, 0xDA) or offset + 2 > len(data):
            break
        segment_length = struct.unpack(">H", data[offset : offset + 2])[0]
        if segment_length < 2 or offset + segment_length > len(data):
            raise VividError("JPEG contains a truncated segment")
        if marker in start_of_frame:
            if segment_length < 7:
                raise VividError("JPEG frame header is too short")
            height, width = struct.unpack(">HH", data[offset + 3 : offset + 7])
            return width, height
        offset += segment_length
    raise VividError("JPEG dimensions were not found")


def fitted_cells(
    image: ImageInfo, display: Dict[int, Any], scale: float
) -> Tuple[int, int]:
    grid_columns = _positive_int(display, 7, "grid columns")
    grid_rows = _positive_int(display, 8, "grid rows")
    cell_width = _positive_int(display, 9, "cell width")
    cell_height = _positive_int(display, 10, "cell height")
    desired_width = max(1.0, image.width * scale)
    desired_height = max(1.0, image.height * scale)
    if not math.isfinite(desired_width) or not math.isfinite(desired_height):
        raise VividError("scaled image dimensions are too large")
    maximum_width = max(1, grid_columns - 4) * cell_width
    maximum_height = max(1, grid_rows - 2) * cell_height
    fit = min(maximum_width / desired_width, maximum_height / desired_height, 1.0)
    target_width = max(1, round(desired_width * fit))
    target_height = max(1, round(desired_height * fit))
    return math.ceil(target_width / cell_width), math.ceil(target_height / cell_height)


def _positive_int(mapping: Dict[int, Any], key: int, name: str) -> int:
    value = mapping.get(key)
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        raise VividError(f"presenter supplied invalid {name}")
    return value


def _unsigned_int(mapping: Dict[int, Any], key: int, name: str) -> int:
    value = mapping.get(key)
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise VividError(f"presenter supplied invalid {name}")
    return value


class VividImageClient:
    def __init__(self, endpoint: str, token: str) -> None:
        if len(token) != 64:
            raise VividError(
                "VIVID_TOKEN must contain exactly 64 hexadecimal characters"
            )
        try:
            self.token_bytes = bytes.fromhex(token)
        except ValueError as error:
            raise VividError("VIVID_TOKEN is not hexadecimal") from error
        if len(self.token_bytes) != 32:
            raise VividError("VIVID_TOKEN does not decode to 32 bytes")

        self.endpoint = endpoint
        self.control = Connection(endpoint, CONTROL, CONTROL_LIMIT)
        self.request_counter = 1
        self.object_counter = 0
        self.root_context_id = 0
        self.display: Dict[int, Any] = {}
        self.session_tag = b""
        self.accepted_features: Set[int] = set()
        self._hello(token)

    def close(self) -> None:
        self.control.close()

    def _request_id(self) -> int:
        self.request_counter += 1
        return self.request_counter

    def _object_id(self) -> int:
        self.object_counter += 1
        return self.object_counter

    def _hello(self, token: str) -> None:
        required = [
            FEATURE_RASTER_RGBA8,
            FEATURE_SCENE_TRANSACTIONS,
            FEATURE_GRID_CELL_NODES,
            FEATURE_CREDIT_FLOW_CONTROL,
            FEATURE_TEXT_ANCHORS,
        ]
        optional = [FEATURE_ENCODED_IMAGE, FEATURE_OBSERVABILITY_CORE]
        payload = {
            0: 1,
            1: 1,
            2: 1,
            3: 1,
            4: token,
            5: "vivid-python-image-demo",
            6: "demo",
            7: required,
            8: optional,
            9: CONTROL_LIMIT,
        }
        self.control.send_record(HELLO, 0, envelope(1, payload))
        _, welcome = self._wait_for({WELCOME}, request_id=1)
        self.display = welcome
        _positive_int(welcome, 0, "session ID")
        self.root_context_id = _positive_int(welcome, 2, "root context ID")
        if welcome.get(13) != 1 or welcome.get(14) != 1:
            raise VividError("presenter selected an unsupported Vivid version")
        session_tag = welcome.get(1)
        features = welcome.get(15)
        if not isinstance(session_tag, bytes) or len(session_tag) != 16:
            raise VividError("presenter supplied an invalid session tag")
        if not isinstance(features, list) or any(
            not isinstance(item, int) for item in features
        ):
            raise VividError("presenter supplied an invalid feature list")
        self.session_tag = session_tag
        self.accepted_features = set(features)
        missing = set(required + optional) - self.accepted_features
        if missing:
            raise VividError(
                f"presenter does not support required demo features: {sorted(missing)}"
            )
        self.control.send_limit = min(
            CONTROL_LIMIT, _positive_int(welcome, 11, "control-body limit")
        )

    def _wait_for(
        self,
        accepted_types: Set[int],
        request_id: Optional[int] = None,
        object_id: Optional[int] = None,
    ) -> Tuple[Record, Dict[int, Any]]:
        while True:
            record = self.control.read_record()
            reply_request, payload = decode_envelope(record.body)
            if record.record_type == DISPLAY_CHANGED:
                self.display.update(
                    {
                        4: payload.get(0),
                        5: payload.get(1),
                        6: payload.get(2),
                        7: payload.get(3),
                        8: payload.get(4),
                        9: payload.get(5),
                        10: payload.get(6),
                    }
                )
                continue
            if record.record_type == ERROR:
                code = payload.get(0, 0)
                failed_request = payload.get(1, reply_request)
                diagnostic = payload.get(5, "unspecified presenter error")
                raise PresenterError(int(code), int(failed_request), str(diagnostic))
            if record.record_type == SOURCE_LOST:
                code = payload.get(1, 0)
                diagnostic = payload.get(2, "source was lost")
                raise PresenterError(int(code), reply_request, str(diagnostic))
            if record.record_type not in accepted_types:
                continue
            if request_id is not None and reply_request != request_id:
                continue
            if object_id is not None and record.object_id != object_id:
                continue
            return record, payload

    def create_anchor(self) -> int:
        if os.environ.get("TMUX") or os.environ.get("STY"):
            raise VividError(
                "run this demo directly in Vivido; tmux/screen anchors are unsupported"
            )
        anchor_id = secrets.randbits(64)
        while anchor_id == 0:
            anchor_id = secrets.randbits(64)
        key = hmac.new(
            self.token_bytes, b"VIVID-ANCHOR-KEY-V2" + self.session_tag, hashlib.sha256
        ).digest()
        auth = hmac.new(
            key,
            b"VIVID-ANCHOR-V2" + self.session_tag + struct.pack(">Q", anchor_id),
            hashlib.sha256,
        ).digest()[:16]
        tag_text = base64.urlsafe_b64encode(self.session_tag).rstrip(b"=")
        auth_text = base64.urlsafe_b64encode(auth).rstrip(b"=")
        marker = (
            b"\x1b_VIVID;2;A;"
            + tag_text
            + b";"
            + f"{anchor_id:016x}".encode("ascii")
            + b";"
            + auth_text
            + b"\x1b\\"
        )
        conpty = os.environ.get("VIVID_ANCHOR_TRANSPORT") == "conpty"
        if conpty:
            marker = marker[2:-2] + b";VIVID-END"
        _write_terminal(marker)
        if not conpty:
            _, payload = self._wait_for({ANCHOR_READY}, object_id=anchor_id)
            if payload.get(0) != anchor_id:
                raise VividError("presenter acknowledged the wrong text anchor")
        return anchor_id

    def create_image(self, image: ImageInfo) -> SourceReady:
        source_id = self._object_id()
        request_id = self._request_id()
        payload = {
            0: source_id,
            1: image.encoding,
            2: image.width,
            3: image.height,
            4: len(image.data),
            5: hashlib.sha256(image.data).digest(),
            6: COLOR_SPACE_SRGB,
            7: RETENTION_DECODED_SOURCE,
        }
        self.control.send_record(CREATE_IMAGE, source_id, envelope(request_id, payload))
        _, ready = self._wait_for({SOURCE_READY}, request_id=request_id)
        ticket = ready.get(1)
        if (
            ready.get(0) != source_id
            or not isinstance(ticket, bytes)
            or len(ticket) != 32
        ):
            raise VividError("presenter supplied an invalid image media ticket")
        return SourceReady(
            source_id,
            ticket,
            _positive_int(ready, 2, "byte credits"),
            _positive_int(ready, 3, "packet credits"),
            _positive_int(ready, 5, "media-body limit"),
        )

    def place_image(
        self, source_id: int, anchor_id: int, columns: int, rows: int
    ) -> None:
        node_id = self._object_id()
        transaction_id = self._object_id()
        begin_request = self._request_id()
        self.control.send_record(
            BEGIN_TXN,
            0,
            envelope(begin_request, {0: transaction_id}, transaction_id=transaction_id),
        )
        node_request = self._request_id()
        node = {
            0: node_id,
            1: source_id,
            2: self.root_context_id,
            3: COORDINATE_ANCHOR_CELL,
            4: 0,
            5: 0,
            6: columns << 32,
            7: rows << 32,
            8: FIT_CONTAIN,
            9: SAMPLING_LINEAR,
            10: TEXT_LAYER_BETWEEN_BACKGROUND_AND_GLYPH,
            11: 0,
            12: BLEND_SOURCE_OVER,
            13: True,
            14: anchor_id,
        }
        self.control.send_record(
            CREATE_NODE,
            node_id,
            envelope(node_request, node, transaction_id=transaction_id),
        )
        for attempt in range(4):
            commit_request = self._request_id()
            commit = {0: PRESENT_NEXT_COMPOSITOR_FRAME, 1: True}
            self.control.send_record(
                COMMIT_TXN,
                0,
                envelope(
                    commit_request,
                    commit,
                    transaction_id=transaction_id,
                    expected_generation=_unsigned_int(
                        self.display, 4, "display generation"
                    ),
                ),
            )
            try:
                self._wait_for({OK, PRESENTED}, request_id=commit_request)
                return
            except PresenterError as error:
                if error.code != ERROR_STALE_DISPLAY_GENERATION or attempt == 3:
                    raise
        raise VividError("could not commit image placement")

    def send_image(self, source: SourceReady, image: ImageInfo) -> None:
        if len(image.data) > source.max_media_body:
            raise VividError("image exceeds the source's negotiated media-body limit")
        if len(image.data) > source.byte_credits or source.packet_credits < 1:
            raise VividError("presenter did not grant enough initial image credit")
        media = Connection(self.endpoint, BLOB, MEDIA_LIMIT)
        try:
            media.send_record(
                ATTACH_CHANNEL, source.source_id, envelope(0, {0: source.ticket})
            )
            media.send_limit = source.max_media_body
            media.send_record(IMAGE_DATA, source.source_id, image.data)
            self._wait_for({CREDIT}, object_id=source.source_id)
        finally:
            media.close()

    def wait_until_visible(self, source_id: int, timeout_seconds: float = 10.0) -> None:
        if FEATURE_OBSERVABILITY_CORE not in self.accepted_features:
            return
        request_id = self._request_id()
        timeout_us = int(timeout_seconds * 1_000_000)
        self.control.send_record(
            WAIT_SOURCE,
            source_id,
            envelope(
                request_id,
                {
                    0: source_id,
                    1: WAIT_FIRST_VISIBLE_PRESENTATION,
                    3: timeout_us,
                },
            ),
        )
        _, satisfied = self._wait_for(
            {WAIT_SATISFIED}, request_id=request_id, object_id=source_id
        )
        if (
            satisfied.get(0) != source_id
            or satisfied.get(2) != WAIT_FIRST_VISIBLE_PRESENTATION
        ):
            raise VividError("presenter satisfied the wrong image milestone")

    def goodbye(self) -> None:
        request_id = self._request_id()
        self.control.send_record(GOODBYE, 0, envelope(request_id, {}))
        self._wait_for({OK}, request_id=request_id)


def _write_terminal(data: bytes) -> None:
    stream = getattr(sys.stdout, "buffer", None)
    if stream is not None:
        stream.write(data)
        stream.flush()
    else:
        sys.stdout.write(data.decode("ascii"))
        sys.stdout.flush()


def display_image(path: Path, scale: float) -> None:
    endpoint = os.environ.get("VIVID_ENDPOINT")
    token = os.environ.get("VIVID_TOKEN")
    if not endpoint or not token:
        raise VividError(
            "VIVID_ENDPOINT and VIVID_TOKEN are required; run inside Vivido"
        )
    if not sys.stdout.isatty():
        raise VividError("stdout must be attached to the Vivido terminal")
    image = inspect_image(path)
    client = VividImageClient(endpoint, token)
    try:
        columns, rows = fitted_cells(image, client.display, scale)
        anchor_id = client.create_anchor()
        source = client.create_image(image)
        client.place_image(source.source_id, anchor_id, columns, rows)
        _write_terminal(b"\r\n" * rows)
        client.send_image(source, image)
        client.wait_until_visible(source.source_id)
        client.goodbye()
    finally:
        client.close()


def parse_args(argv: Optional[Sequence[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Display a PNG or JPEG in Vivido using Vivid Protocol directly."
    )
    parser.add_argument("image", type=Path, help="PNG or JPEG file to display")
    parser.add_argument(
        "--scale",
        type=float,
        default=1.0,
        help="scale relative to the image's natural pixel size (default: 1.0)",
    )
    args = parser.parse_args(argv)
    if not math.isfinite(args.scale) or args.scale <= 0:
        parser.error("--scale must be a positive finite number")
    return args


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = parse_args(argv)
    try:
        display_image(args.image, args.scale)
    except (OSError, VividError) as error:
        print(f"vivid_image: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
