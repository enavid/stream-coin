"""
Consumer unit tests — decode path only.

The Kafka connection is not exercised here; only the protobuf decode logic
is tested. Integration tests that require a running Kafka broker are tagged
with @pytest.mark.integration and excluded from the default pytest run.
"""
from unittest.mock import MagicMock, patch
import pytest

from stream_coin.consumer import CandleConsumer, SignalConsumer
from stream_coin.proto.candle import Candle
from stream_coin.proto.signal import Signal


# ---------------------------------------------------------------------------
# CandleConsumer.decode (static method — no Kafka)

def test_candle_consumer_decode_valid_bytes():
    c = Candle(
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
    decoded = CandleConsumer.decode(bytes(c))

    assert decoded.exchange == "tabdeal"
    assert decoded.pair == "USDT/IRT"
    assert decoded.interval == "1m"
    assert decoded.time == 1_700_000_000_000
    assert decoded.open == 570_000


def test_candle_consumer_decode_empty_bytes_returns_defaults():
    decoded = CandleConsumer.decode(b"")
    assert decoded.exchange == ""
    assert decoded.open == 0


def test_candle_consumer_decode_returns_candle_instance():
    raw = bytes(Candle(exchange="x", open=1))
    result = CandleConsumer.decode(raw)
    assert isinstance(result, Candle)


# ---------------------------------------------------------------------------
# SignalConsumer.decode (static method — no Kafka)

def test_signal_consumer_decode_valid_bytes():
    s = Signal(
        signal_id="test-uuid",
        strategy_id="spread_threshold",
        exchange="coinex",
        pair="BTC/USDT",
        action="buy",
        confidence=0.9,
        timestamp=1_700_000_000_000,
        stop_loss=None,
        take_profit=600_000,
    )
    decoded = SignalConsumer.decode(bytes(s))

    assert decoded.signal_id == "test-uuid"
    assert decoded.action == "buy"
    assert decoded.confidence == 0.9
    assert decoded.stop_loss is None
    assert decoded.take_profit == 600_000


def test_signal_consumer_decode_returns_signal_instance():
    raw = bytes(Signal(exchange="x", action="sell"))
    result = SignalConsumer.decode(raw)
    assert isinstance(result, Signal)


# ---------------------------------------------------------------------------
# Consumer poll — mock the confluent_kafka.Consumer

def _make_mock_message(value: bytes, error=None):
    msg = MagicMock()
    msg.error.return_value = error
    msg.value.return_value = value
    return msg


def test_candle_consumer_poll_decodes_message():
    raw_candle = bytes(Candle(exchange="tabdeal", pair="USDT/IRT", open=500_000))

    with patch("stream_coin.consumer.ConfluentConsumer") as MockKafka:
        mock_kafka = MagicMock()
        MockKafka.return_value = mock_kafka
        mock_kafka.poll.return_value = _make_mock_message(raw_candle)

        consumer = CandleConsumer(bootstrap_servers="localhost:9092", group_id="test")
        consumer._kafka = mock_kafka  # inject mock directly

        result = consumer.poll(timeout=0.1)

    assert result is not None
    assert result.exchange == "tabdeal"
    assert result.open == 500_000


def test_candle_consumer_poll_returns_none_when_kafka_returns_none():
    with patch("stream_coin.consumer.ConfluentConsumer") as MockKafka:
        mock_kafka = MagicMock()
        MockKafka.return_value = mock_kafka
        mock_kafka.poll.return_value = None

        consumer = CandleConsumer(bootstrap_servers="localhost:9092", group_id="test")
        consumer._kafka = mock_kafka

        result = consumer.poll(timeout=0.1)

    assert result is None


def test_candle_consumer_poll_returns_none_on_kafka_error():
    error = MagicMock()
    error.__bool__ = lambda self: True  # truthy error

    with patch("stream_coin.consumer.ConfluentConsumer") as MockKafka:
        mock_kafka = MagicMock()
        MockKafka.return_value = mock_kafka
        mock_kafka.poll.return_value = _make_mock_message(b"", error=error)

        consumer = CandleConsumer(bootstrap_servers="localhost:9092", group_id="test")
        consumer._kafka = mock_kafka

        result = consumer.poll(timeout=0.1)

    assert result is None


def test_signal_consumer_poll_decodes_message():
    raw_signal = bytes(Signal(
        exchange="tabdeal",
        action="buy",
        confidence=0.85,
        stop_loss=None,
    ))

    with patch("stream_coin.consumer.ConfluentConsumer") as MockKafka:
        mock_kafka = MagicMock()
        MockKafka.return_value = mock_kafka
        mock_kafka.poll.return_value = _make_mock_message(raw_signal)

        consumer = SignalConsumer(bootstrap_servers="localhost:9092", group_id="test")
        consumer._kafka = mock_kafka

        result = consumer.poll(timeout=0.1)

    assert result is not None
    assert result.action == "buy"
    assert abs(result.confidence - 0.85) < 1e-9


# ---------------------------------------------------------------------------
# Context manager lifecycle

def test_candle_consumer_context_manager_opens_and_closes():
    with patch("stream_coin.consumer.ConfluentConsumer") as MockKafka:
        mock_kafka = MagicMock()
        MockKafka.return_value = mock_kafka

        with CandleConsumer(bootstrap_servers="localhost:9092", group_id="cg") as consumer:
            consumer.subscribe("candles.proto")
            mock_kafka.subscribe.assert_called_once_with(["candles.proto"])

        mock_kafka.close.assert_called_once()


def test_candle_consumer_context_manager_closes_on_exception():
    with patch("stream_coin.consumer.ConfluentConsumer") as MockKafka:
        mock_kafka = MagicMock()
        MockKafka.return_value = mock_kafka

        with pytest.raises(RuntimeError):
            with CandleConsumer(bootstrap_servers="localhost:9092", group_id="cg") as _consumer:
                raise RuntimeError("forced failure")

        mock_kafka.close.assert_called_once()
