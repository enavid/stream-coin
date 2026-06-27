"""
Kafka consumers for stream-coin candle/signal Protobuf streams.

CandleConsumer and SignalConsumer read from the ``candles.proto`` and
``signals.proto`` topics respectively and decode each message using the
betterproto-generated Candle/Signal dataclasses.

Usage (context-manager pattern closes the underlying consumer on exit):

    with CandleConsumer("kafka:9092", group_id="my-strategy") as consumer:
        consumer.subscribe("candles.proto")
        while True:
            candle = consumer.poll(timeout=1.0)
            if candle:
                process(candle)

For bulk ingestion, call ``subscribe()`` before entering the polling loop.
``decode()`` is a static method; use it directly when you already have raw
bytes (e.g., from a test fixture or a custom consumer).
"""
import logging
from typing import Optional

from confluent_kafka import Consumer as ConfluentConsumer  # noqa: F401  (re-exported for mocking)
from confluent_kafka import KafkaError

from .proto.candle import Candle
from .proto.signal import Signal

logger = logging.getLogger(__name__)


class CandleConsumer:
    """Kafka consumer that decodes protobuf Candle messages from a topic.

    The class owns a ``confluent_kafka.Consumer`` instance (created on
    construction). Use it as a context manager to ensure ``close()`` is always
    called, even when an exception occurs.
    """

    def __init__(self, bootstrap_servers: str, group_id: str) -> None:
        config = {
            "bootstrap.servers": bootstrap_servers,
            "group.id": group_id,
            "auto.offset.reset": "earliest",
            "enable.auto.commit": True,
        }
        self._kafka = ConfluentConsumer(config)

    def __enter__(self) -> "CandleConsumer":
        return self

    def __exit__(self, *args: object) -> None:
        self._kafka.close()
        logger.info("candle consumer closed")

    def subscribe(self, topic: str) -> None:
        self._kafka.subscribe([topic])
        logger.info("candle consumer subscribed to topic=%s", topic)

    def poll(self, timeout: float = 1.0) -> Optional[Candle]:
        """Poll once and return a decoded Candle, or None on timeout/error."""
        msg = self._kafka.poll(timeout)
        if msg is None:
            return None
        if msg.error():
            err = msg.error()
            if err.code() == KafkaError._PARTITION_EOF:
                return None
            logger.error("candle consumer kafka error: %s", err)
            return None
        try:
            return self.decode(msg.value())
        except Exception:
            logger.exception(
                "failed to decode candle message (offset=%s topic=%s)",
                msg.offset(),
                msg.topic(),
            )
            return None

    @staticmethod
    def decode(raw_bytes: bytes) -> Candle:
        """Decode raw protobuf bytes into a Candle dataclass."""
        return Candle().parse(raw_bytes)


class SignalConsumer:
    """Kafka consumer that decodes protobuf Signal messages from a topic.

    Mirrors CandleConsumer in every way; see its docstring for usage.
    """

    def __init__(self, bootstrap_servers: str, group_id: str) -> None:
        config = {
            "bootstrap.servers": bootstrap_servers,
            "group.id": group_id,
            "auto.offset.reset": "earliest",
            "enable.auto.commit": True,
        }
        self._kafka = ConfluentConsumer(config)

    def __enter__(self) -> "SignalConsumer":
        return self

    def __exit__(self, *args: object) -> None:
        self._kafka.close()
        logger.info("signal consumer closed")

    def subscribe(self, topic: str) -> None:
        self._kafka.subscribe([topic])
        logger.info("signal consumer subscribed to topic=%s", topic)

    def poll(self, timeout: float = 1.0) -> Optional[Signal]:
        """Poll once and return a decoded Signal, or None on timeout/error."""
        msg = self._kafka.poll(timeout)
        if msg is None:
            return None
        if msg.error():
            err = msg.error()
            if err.code() == KafkaError._PARTITION_EOF:
                return None
            logger.error("signal consumer kafka error: %s", err)
            return None
        try:
            return self.decode(msg.value())
        except Exception:
            logger.exception(
                "failed to decode signal message (offset=%s topic=%s)",
                msg.offset(),
                msg.topic(),
            )
            return None

    @staticmethod
    def decode(raw_bytes: bytes) -> Signal:
        """Decode raw protobuf bytes into a Signal dataclass."""
        return Signal().parse(raw_bytes)
