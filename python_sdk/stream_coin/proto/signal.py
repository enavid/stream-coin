"""
Protobuf Signal message — mirrors proto/signal.proto (stream_coin.v1.Signal).

stop_loss and take_profit are proto3 ``optional`` fields: absence on the wire
decodes as None (not 0). The ``group`` parameter implements this using proto3's
synthetic-oneof mechanism so None vs 0 round-trips correctly, matching prost's
Option<u64> semantics on the Rust producer side.
"""
from dataclasses import dataclass
from typing import Optional
import betterproto


@dataclass
class Signal(betterproto.Message):
    """Wire contract for the Kafka ``signals.proto`` topic."""

    signal_id: str = betterproto.string_field(1)
    strategy_id: str = betterproto.string_field(2)
    exchange: str = betterproto.string_field(3)
    pair: str = betterproto.string_field(4)
    action: str = betterproto.string_field(5)
    confidence: float = betterproto.double_field(6)
    timestamp: int = betterproto.int64_field(7)
    stop_loss: Optional[int] = betterproto.uint64_field(8, group="_stop_loss")
    take_profit: Optional[int] = betterproto.uint64_field(9, group="_take_profit")
