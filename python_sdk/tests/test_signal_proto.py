"""
Signal protobuf decode tests.

Verifies that the betterproto Signal class correctly decodes wire bytes produced
by the Rust prost encoder (stream_coin.v1.Signal from proto/signal.proto).
The trickiest invariant here is proto3 `optional` semantics: stop_loss/take_profit
absent from the wire must decode as None, not 0; an explicit 0 value must decode
as 0 (not None), just as prost's Option<u64> distinguishes Some(0) from None.
"""
import pytest

from stream_coin.proto.signal import Signal


# ---------------------------------------------------------------------------
# Helpers — minimal wire encoder, mirroring the field layout of signal.proto

def _varint(v: int) -> bytes:
    result = []
    if v < 0:
        v &= 0xFFFFFFFFFFFFFFFF
    while v > 0x7F:
        result.append((v & 0x7F) | 0x80)
        v >>= 7
    result.append(v & 0x7F)
    return bytes(result)


def _str_field(n: int, s: str) -> bytes:
    tag = (n << 3) | 2
    enc = s.encode()
    return _varint(tag) + _varint(len(enc)) + enc


def _i64_field(n: int, v: int) -> bytes:
    return _varint((n << 3) | 0) + _varint(v if v >= 0 else v & 0xFFFFFFFFFFFFFFFF)


def _u64_field(n: int, v: int) -> bytes:
    return _varint((n << 3) | 0) + _varint(v)


def _f64_field(n: int, v: float) -> bytes:
    import struct
    return _varint((n << 3) | 1) + struct.pack("<d", v)


def _build_signal_bytes(
    signal_id: str = "",
    strategy_id: str = "",
    exchange: str = "",
    pair: str = "",
    action: str = "",
    confidence: float = 0.0,
    timestamp: int = 0,
    stop_loss: int | None = None,
    take_profit: int | None = None,
) -> bytes:
    buf = b""
    if signal_id:
        buf += _str_field(1, signal_id)
    if strategy_id:
        buf += _str_field(2, strategy_id)
    if exchange:
        buf += _str_field(3, exchange)
    if pair:
        buf += _str_field(4, pair)
    if action:
        buf += _str_field(5, action)
    if confidence != 0.0:
        buf += _f64_field(6, confidence)
    if timestamp != 0:
        buf += _i64_field(7, timestamp)
    # Proto3 `optional` fields: encode only when not None — even 0 is encoded
    if stop_loss is not None:
        buf += _u64_field(8, stop_loss)
    if take_profit is not None:
        buf += _u64_field(9, take_profit)
    return buf


# ---------------------------------------------------------------------------
# Basic field decode

def test_signal_decodes_all_required_fields():
    raw = _build_signal_bytes(
        signal_id="550e8400-e29b-41d4-a716-446655440000",
        strategy_id="spread_threshold",
        exchange="tabdeal",
        pair="USDT/IRT",
        action="buy",
        confidence=0.873456789,
        timestamp=1_700_000_000_000,
    )
    s = Signal().parse(raw)

    assert s.signal_id == "550e8400-e29b-41d4-a716-446655440000"
    assert s.strategy_id == "spread_threshold"
    assert s.exchange == "tabdeal"
    assert s.pair == "USDT/IRT"
    assert s.action == "buy"
    assert abs(s.confidence - 0.873456789) < 1e-9, "double precision must survive"
    assert s.timestamp == 1_700_000_000_000


# ---------------------------------------------------------------------------
# Optional field semantics — the hardest invariant in proto3

def test_signal_stop_loss_absent_decodes_as_none():
    """When stop_loss is absent on the wire, Python must see None, not 0."""
    raw = _build_signal_bytes(exchange="x", action="buy", stop_loss=None)
    s = Signal().parse(raw)
    assert s.stop_loss is None, f"expected None, got {s.stop_loss}"


def test_signal_take_profit_absent_decodes_as_none():
    raw = _build_signal_bytes(exchange="x", action="sell", take_profit=None)
    s = Signal().parse(raw)
    assert s.take_profit is None


def test_signal_stop_loss_explicit_zero_decodes_as_zero_not_none():
    """proto3 optional Some(0) must round-trip as 0, not collapse to None."""
    raw = _build_signal_bytes(exchange="x", action="buy", stop_loss=0)
    s = Signal().parse(raw)
    assert s.stop_loss == 0, "stop_loss=0 must be 0, not None"


def test_signal_take_profit_explicit_zero_decodes_as_zero_not_none():
    raw = _build_signal_bytes(exchange="x", action="buy", take_profit=0)
    s = Signal().parse(raw)
    assert s.take_profit == 0


def test_signal_both_optional_fields_present():
    raw = _build_signal_bytes(
        exchange="tabdeal",
        action="buy",
        stop_loss=173_460,
        take_profit=184_080,
    )
    s = Signal().parse(raw)
    assert s.stop_loss == 173_460
    assert s.take_profit == 184_080


def test_signal_max_uint64_stop_loss():
    max_u64 = 0xFFFFFFFFFFFFFFFF
    raw = _build_signal_bytes(exchange="x", action="sell", stop_loss=max_u64)
    s = Signal().parse(raw)
    assert s.stop_loss == max_u64, "uint64 max must not truncate"


# ---------------------------------------------------------------------------
# Action string variants

@pytest.mark.parametrize("action", ["buy", "sell", "hold"])
def test_signal_all_action_strings_roundtrip(action: str):
    raw = _build_signal_bytes(exchange="x", action=action)
    s = Signal().parse(raw)
    assert s.action == action


# ---------------------------------------------------------------------------
# Confidence precision (double, not float)

def test_signal_confidence_precision_preserved():
    precise = 0.8734567890123456
    raw = _build_signal_bytes(exchange="x", action="buy", confidence=precise)
    s = Signal().parse(raw)
    assert s.confidence == precise, "double confidence must survive bit-for-bit"


def test_signal_confidence_at_boundaries():
    for boundary in [0.0, 1.0]:
        raw = _build_signal_bytes(exchange="x", action="buy", confidence=boundary)
        s = Signal().parse(raw)
        assert s.confidence == boundary


# ---------------------------------------------------------------------------
# Negative timestamp (same as Candle: int64, pre-epoch is valid)

def test_signal_negative_timestamp_preserved():
    ts = -1_000_000_000
    raw = _build_signal_bytes(exchange="x", action="hold", timestamp=ts)
    s = Signal().parse(raw)
    assert s.timestamp == ts


# ---------------------------------------------------------------------------
# Roundtrip: betterproto → bytes → betterproto

def test_signal_betterproto_encode_decode_roundtrip():
    original = Signal(
        signal_id="abc-123",
        strategy_id="rsi_cross",
        exchange="coinex",
        pair="ETH/USDT",
        action="sell",
        confidence=0.91,
        timestamp=1_710_000_000_000,
        stop_loss=None,
        take_profit=500_000,
    )
    decoded = Signal().parse(bytes(original))

    assert decoded.signal_id == original.signal_id
    assert decoded.strategy_id == original.strategy_id
    assert decoded.exchange == original.exchange
    assert decoded.pair == original.pair
    assert decoded.action == original.action
    assert decoded.confidence == original.confidence
    assert decoded.timestamp == original.timestamp
    assert decoded.stop_loss is None
    assert decoded.take_profit == original.take_profit


def test_signal_roundtrip_with_stop_loss_zero():
    original = Signal(
        exchange="tabdeal",
        action="buy",
        stop_loss=0,
        take_profit=None,
    )
    decoded = Signal().parse(bytes(original))
    assert decoded.stop_loss == 0
    assert decoded.take_profit is None
