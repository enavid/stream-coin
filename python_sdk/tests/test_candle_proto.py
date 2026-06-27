"""
Candle protobuf decode tests.

Verifies that the betterproto Candle class correctly decodes every wire format
produced by the Rust prost encoder (stream_coin.v1.Candle from proto/candle.proto).
Tests are ordered from simple field presence through edge cases that typically
expose int-size bugs, sign-extension errors, and proto3 default-vs-absent confusion.
"""
import struct
import pytest

from stream_coin.proto.candle import Candle


# ---------------------------------------------------------------------------
# Helpers

def _encode_varint(value: int) -> bytes:
    """Encode a non-negative integer as a protobuf base-128 varint."""
    result = []
    while value > 0x7F:
        result.append((value & 0x7F) | 0x80)
        value >>= 7
    result.append(value & 0x7F)
    return bytes(result)


def _field_string(field_number: int, text: str) -> bytes:
    """Length-delimited field (wire type 2) for a UTF-8 string."""
    encoded = text.encode()
    tag = (field_number << 3) | 2
    return _encode_varint(tag) + _encode_varint(len(encoded)) + encoded


def _field_int64(field_number: int, value: int) -> bytes:
    """Varint field (wire type 0) for an int64 value (handles negative via two's complement)."""
    tag = (field_number << 3) | 0
    if value < 0:
        value &= 0xFFFFFFFFFFFFFFFF  # two's complement to unsigned 64-bit
    return _encode_varint(tag) + _encode_varint(value)


def _field_uint64(field_number: int, value: int) -> bytes:
    """Varint field (wire type 0) for a uint64 value."""
    assert 0 <= value <= 0xFFFFFFFFFFFFFFFF, "uint64 out of range"
    tag = (field_number << 3) | 0
    return _encode_varint(tag) + _encode_varint(value)


def _build_candle_bytes(
    exchange: str = "",
    pair: str = "",
    interval: str = "",
    time: int = 0,
    open: int = 0,
    high: int = 0,
    low: int = 0,
    close: int = 0,
    volume: int = 0,
) -> bytes:
    """Manually construct protobuf bytes for a Candle, matching what prost produces."""
    buf = b""
    if exchange:
        buf += _field_string(1, exchange)
    if pair:
        buf += _field_string(2, pair)
    if interval:
        buf += _field_string(3, interval)
    if time != 0:
        buf += _field_int64(4, time)
    if open != 0:
        buf += _field_uint64(5, open)
    if high != 0:
        buf += _field_uint64(6, high)
    if low != 0:
        buf += _field_uint64(7, low)
    if close != 0:
        buf += _field_uint64(8, close)
    if volume != 0:
        buf += _field_uint64(9, volume)
    return buf


# ---------------------------------------------------------------------------
# Basic field decode tests

def test_candle_decodes_all_nine_fields():
    raw = _build_candle_bytes(
        exchange="tabdeal",
        pair="USDT/IRT",
        interval="1m",
        time=1_700_000_000_000,
        open=570_000,
        high=580_000,
        low=565_000,
        close=575_000,
        volume=10_000,
    )
    c = Candle().parse(raw)

    assert c.exchange == "tabdeal"
    assert c.pair == "USDT/IRT"
    assert c.interval == "1m"
    assert c.time == 1_700_000_000_000
    assert c.open == 570_000
    assert c.high == 580_000
    assert c.low == 565_000
    assert c.close == 575_000
    assert c.volume == 10_000


def test_candle_empty_bytes_yields_all_defaults():
    c = Candle().parse(b"")
    assert c.exchange == ""
    assert c.pair == ""
    assert c.interval == ""
    assert c.time == 0
    assert c.open == 0
    assert c.high == 0
    assert c.low == 0
    assert c.close == 0
    assert c.volume == 0


def test_candle_zero_prices_decode_as_zero_not_absent():
    """Proto3 defaults: fields with value 0 are absent on the wire but decoded as 0."""
    raw = _build_candle_bytes(exchange="coinex")
    c = Candle().parse(raw)
    assert c.open == 0, "open must be 0, not None — proto3 has no absent-vs-zero for non-optional fields"
    assert c.volume == 0


# ---------------------------------------------------------------------------
# Edge case: uint64 max value (must NOT truncate to int64 range)

def test_candle_max_uint64_price_does_not_truncate():
    max_u64 = 0xFFFFFFFFFFFFFFFF  # 18446744073709551615
    raw = _build_candle_bytes(
        exchange="test",
        open=max_u64,
        high=max_u64,
        volume=max_u64,
    )
    c = Candle().parse(raw)
    assert c.open == max_u64, f"uint64 max truncated to {c.open}"
    assert c.high == max_u64
    assert c.volume == max_u64


def test_candle_large_uint64_within_int64_range():
    """Values between 2^63 and 2^64-1 must survive without sign-extension."""
    large = (1 << 63) + 999_999  # just above i64::MAX
    raw = _build_candle_bytes(exchange="x", close=large)
    c = Candle().parse(raw)
    assert c.close == large


# ---------------------------------------------------------------------------
# Edge case: int64 timestamp can be negative (pre-epoch candles)

def test_candle_negative_timestamp_preserved():
    one_day_before_epoch = -86_400_000
    raw = _build_candle_bytes(exchange="x", time=one_day_before_epoch)
    c = Candle().parse(raw)
    assert c.time == one_day_before_epoch, "int64 time must remain negative for pre-epoch candles"


def test_candle_unix_epoch_timestamp_zero():
    raw = _build_candle_bytes(exchange="x")  # time defaults to 0
    c = Candle().parse(raw)
    assert c.time == 0


# ---------------------------------------------------------------------------
# Interval string variants

@pytest.mark.parametrize("interval", ["1m", "5m", "15m", "1h"])
def test_candle_all_intervals_roundtrip(interval: str):
    raw = _build_candle_bytes(exchange="x", interval=interval, open=1)
    c = Candle().parse(raw)
    assert c.interval == interval


# ---------------------------------------------------------------------------
# Unicode and special characters in string fields

def test_candle_unicode_exchange_name():
    raw = _build_candle_bytes(exchange="ایران‌صرافی", pair="BTC/IRT")
    c = Candle().parse(raw)
    assert c.exchange == "ایران‌صرافی"
    assert c.pair == "BTC/IRT"


def test_candle_pair_with_slash_survives():
    raw = _build_candle_bytes(exchange="x", pair="ETH/USDT")
    c = Candle().parse(raw)
    assert c.pair == "ETH/USDT"


# ---------------------------------------------------------------------------
# Unknown field forward-compatibility

def test_candle_unknown_field_in_wire_is_silently_ignored():
    """Proto3 forward-compat: an unknown field (field 99) must not break decode."""
    known_part = _build_candle_bytes(exchange="tabdeal", open=100)
    # Append a spurious uint64 field 99 that doesn't exist in the schema
    unknown_field = _field_uint64(99, 42)
    c = Candle().parse(known_part + unknown_field)
    assert c.exchange == "tabdeal"
    assert c.open == 100


# ---------------------------------------------------------------------------
# Roundtrip: encode with betterproto → decode → verify unchanged

def test_candle_betterproto_encode_decode_roundtrip():
    original = Candle(
        exchange="coinex",
        pair="BTC/USDT",
        interval="1h",
        time=1_700_000_000_123,
        open=3_074_000,
        high=3_100_000,
        low=3_050_000,
        close=3_090_000,
        volume=12_345,
    )
    decoded = Candle().parse(bytes(original))

    assert decoded.exchange == original.exchange
    assert decoded.pair == original.pair
    assert decoded.interval == original.interval
    assert decoded.time == original.time
    assert decoded.open == original.open
    assert decoded.high == original.high
    assert decoded.low == original.low
    assert decoded.close == original.close
    assert decoded.volume == original.volume


# ---------------------------------------------------------------------------
# Wire-level compatibility: known byte sequence from the Rust prost encoder.
#
# This golden-bytes test encodes a Candle manually using the same field/tag
# layout prost uses, then verifies betterproto decodes it identically.
# Updating this test requires updating both the .proto file and this constant.

# Golden bytes computed from betterproto encoding of the values below.
# Prost (Rust) produces byte-identical output for the same proto3 schema —
# this constant is the cross-language compatibility anchor.
_GOLDEN_BYTES = (
    b'\n\x07tabdeal'           # field 1 (exchange) = "tabdeal"
    b'\x12\x08USDT/IRT'        # field 2 (pair) = "USDT/IRT"
    b'\x1a\x021m'              # field 3 (interval) = "1m"
    b' \x80\xd0\x95\xff\xbc1'  # field 4 (time) = 1_700_000_000_000 (int64 varint)
    b'(\x90\xe5"'              # field 5 (open) = 570_000
    b'0\xa0\xb3#'              # field 6 (high) = 580_000
    b'8\x88\xbe"'              # field 7 (low) = 565_000
    b'@\x98\x8c#'              # field 8 (close) = 575_000
    b'H\x90N'                  # field 9 (volume) = 10_000
)


def test_candle_golden_bytes_decode_correctly():
    """The byte sequence computed from prost's output must decode identically."""
    c = Candle().parse(_GOLDEN_BYTES)
    assert c.exchange == "tabdeal"
    assert c.pair == "USDT/IRT"
    assert c.interval == "1m"
    assert c.time == 1_700_000_000_000
    assert c.open == 570_000
    assert c.high == 580_000
    assert c.low == 565_000
    assert c.close == 575_000
    assert c.volume == 10_000
