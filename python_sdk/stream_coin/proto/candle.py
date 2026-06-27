"""
Protobuf Candle message — mirrors proto/candle.proto (stream_coin.v1.Candle).

Field numbers must match the .proto file exactly; changing either requires
regenerating both this file and the Rust prost types, and bumping the Kafka
topic schema version.

Prices and volume are minor-unit integers (never floats) to preserve full
financial precision. Time is the candle open time as Unix milliseconds (UTC);
using int64 means pre-epoch candles (negative timestamps) are representable.
"""
from dataclasses import dataclass
import betterproto


@dataclass
class Candle(betterproto.Message):
    """Wire contract for the Kafka ``candles.proto`` topic."""

    exchange: str = betterproto.string_field(1)
    pair: str = betterproto.string_field(2)
    interval: str = betterproto.string_field(3)
    time: int = betterproto.int64_field(4)
    open: int = betterproto.uint64_field(5)
    high: int = betterproto.uint64_field(6)
    low: int = betterproto.uint64_field(7)
    close: int = betterproto.uint64_field(8)
    volume: int = betterproto.uint64_field(9)
