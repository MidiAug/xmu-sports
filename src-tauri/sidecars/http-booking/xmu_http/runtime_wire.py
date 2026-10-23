"""Small, bounded codec for the WMPF remote-debug CDP envelope.

Wire fields were verified against the upstream WARemoteDebug protobuf encoder.
Only the two CDP message categories are consumed; raw payloads are never logged.
"""
import json
import zlib

MAX_MESSAGE = 4 * 1024 * 1024


def varint(value):
    result = bytearray()
    while value > 127:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return bytes(result)


def integer(field, value):
    return varint(field << 3) + varint(value)


def binary(field, value):
    if isinstance(value, str):
        value = value.encode()
    return varint((field << 3) | 2) + varint(len(value)) + value


def decode(data):
    if len(data) > MAX_MESSAGE:
        raise ValueError("oversized envelope")
    index = 0
    def number():
        nonlocal index
        value = 0
        for shift in range(0, 70, 7):
            if index >= len(data):
                raise ValueError("truncated varint")
            byte = data[index]
            index += 1
            value |= (byte & 127) << shift
            if byte < 128:
                return value
        raise ValueError("invalid varint")
    fields = {}
    while index < len(data):
        tag = number()
        field, wire = tag >> 3, tag & 7
        if not field:
            raise ValueError("invalid field")
        if wire == 0:
            fields[field] = number()
        elif wire == 2:
            size = number()
            if index + size > len(data):
                raise ValueError("truncated field")
            fields[field] = data[index:index + size]
            index += size
        elif wire in (1, 5):
            index += 8 if wire == 1 else 4
            if index > len(data):
                raise ValueError("truncated fixed field")
        else:
            raise ValueError("unsupported wire type")
    return fields


def command_envelope(sequence, command):
    payload = json.dumps(command, ensure_ascii=False, separators=(",", ":"))
    # Upstream uses a small transport operation ID, independent of the CDP ID.
    cdp = integer(1, sequence % 100) + binary(2, payload) + binary(3, "")
    return (integer(1, sequence) + binary(3, "chromeDevtools") + binary(4, cdp)
            + integer(5, 0) + integer(6, 0))


def result_message(raw):
    envelope = decode(raw)
    if envelope.get(3) != b"chromeDevtoolsResult":
        return None
    body = envelope.get(4, b"")
    if envelope.get(5, 0) & 1:
        inflater = zlib.decompressobj()
        body = inflater.decompress(body, MAX_MESSAGE + 1)
        if len(body) > MAX_MESSAGE or inflater.unconsumed_tail:
            raise ValueError("oversized compressed message")
    payload = decode(body).get(2, b"")
    return json.loads(payload) if payload else None
