"""Unit tests for error classes."""

import pytest
from norikv.errors import (
    AlreadyExistsError,
    ConnectionError,
    DeadlineExceededError,
    InvalidArgumentError,
    NoNodesAvailableError,
    NoriKVError,
    NotLeaderError,
    RetryExhaustedError,
    UnavailableError,
    VersionMismatchError,
    from_grpc_error,
)
from norikv.types import Version


class TestNoriKVError:
    """Tests for base NoriKVError class."""

    def test_base_error_creation(self):
        """Should create error with message."""
        error = NoriKVError("test error")
        assert str(error) == "test error"
        assert error.code is None

    def test_base_error_with_code(self):
        """Should create error with message and code."""
        error = NoriKVError("test error", "TEST_CODE")
        assert str(error) == "test error"
        assert error.code == "TEST_CODE"

    def test_base_error_repr(self):
        """Should have readable repr."""
        error = NoriKVError("test error", "TEST_CODE")
        assert "NoriKVError" in repr(error)
        assert "test error" in repr(error)
        assert "TEST_CODE" in repr(error)


class TestNotLeaderError:
    """Tests for NotLeaderError class."""

    def test_not_leader_error_basic(self):
        """Should create NotLeaderError with message."""
        error = NotLeaderError("node is not leader")
        assert str(error) == "node is not leader"
        assert error.code == "NOT_LEADER"
        assert error.leader_hint is None
        assert error.shard_id is None

    def test_not_leader_error_with_hint(self):
        """Should create NotLeaderError with leader hint."""
        error = NotLeaderError(
            "node is not leader",
            leader_hint="localhost:50052",
            shard_id=42,
        )
        assert error.leader_hint == "localhost:50052"
        assert error.shard_id == 42

    def test_not_leader_error_repr(self):
        """Should have readable repr."""
        error = NotLeaderError(
            "not leader", leader_hint="localhost:50052", shard_id=42
        )
        assert "NotLeaderError" in repr(error)
        assert "localhost:50052" in repr(error)
        assert "42" in repr(error)


class TestAlreadyExistsError:
    """Tests for AlreadyExistsError class."""

    def test_already_exists_error(self):
        """Should create AlreadyExistsError with key."""
        key = b"test-key"
        error = AlreadyExistsError("key already exists", key)
        assert str(error) == "key already exists"
        assert error.code == "ALREADY_EXISTS"
        assert error.key == key


class TestVersionMismatchError:
    """Tests for VersionMismatchError class."""

    def test_version_mismatch_error(self):
        """Should create VersionMismatchError with versions."""
        key = b"test-key"
        expected = Version(term=1, index=100)
        actual = Version(term=1, index=101)
        error = VersionMismatchError(
            "version mismatch", key, expected, actual
        )

        assert str(error) == "version mismatch"
        assert error.code == "VERSION_MISMATCH"
        assert error.key == key
        assert error.expected_version == expected
        assert error.actual_version == actual


class TestUnavailableError:
    """Tests for UnavailableError class."""

    def test_unavailable_error(self):
        """Should create UnavailableError."""
        error = UnavailableError("service unavailable")
        assert str(error) == "service unavailable"
        assert error.code == "UNAVAILABLE"


class TestDeadlineExceededError:
    """Tests for DeadlineExceededError class."""

    def test_deadline_exceeded_error(self):
        """Should create DeadlineExceededError with timeout."""
        error = DeadlineExceededError("deadline exceeded", timeout_ms=5000)
        assert str(error) == "deadline exceeded"
        assert error.code == "DEADLINE_EXCEEDED"
        assert error.timeout_ms == 5000


class TestInvalidArgumentError:
    """Tests for InvalidArgumentError class."""

    def test_invalid_argument_error(self):
        """Should create InvalidArgumentError."""
        error = InvalidArgumentError("invalid argument")
        assert str(error) == "invalid argument"
        assert error.code == "INVALID_ARGUMENT"


class TestConnectionError:
    """Tests for ConnectionError class."""

    def test_connection_error_basic(self):
        """Should create ConnectionError with message."""
        error = ConnectionError("connection failed")
        assert str(error) == "connection failed"
        assert error.code == "CONNECTION_ERROR"
        assert error.address is None

    def test_connection_error_with_address(self):
        """Should create ConnectionError with address."""
        error = ConnectionError(
            "connection failed", address="localhost:50051"
        )
        assert error.address == "localhost:50051"


class TestNoNodesAvailableError:
    """Tests for NoNodesAvailableError class."""

    def test_no_nodes_error_default(self):
        """Should create NoNodesAvailableError with default message."""
        error = NoNodesAvailableError()
        assert "No nodes available" in str(error)
        assert error.code == "NO_NODES_AVAILABLE"

    def test_no_nodes_error_custom(self):
        """Should create NoNodesAvailableError with custom message."""
        error = NoNodesAvailableError("custom message")
        assert str(error) == "custom message"


class TestRetryExhaustedError:
    """Tests for RetryExhaustedError class."""

    def test_retry_exhausted_error(self):
        """Should create RetryExhaustedError with attempts."""
        last_error = Exception("last error")
        error = RetryExhaustedError(
            "retry exhausted", attempts=3, last_error=last_error
        )

        assert str(error) == "retry exhausted"
        assert error.code == "RETRY_EXHAUSTED"
        assert error.attempts == 3
        assert error.last_error is last_error


class TestFromGrpcError:
    """Tests for from_grpc_error function."""

    def test_unavailable_error_mapping(self):
        """Should map gRPC UNAVAILABLE to UnavailableError."""

        class MockGrpcError:
            def code(self):
                class Code:
                    value = [14]

                return Code()

            def details(self):
                return "service unavailable"

        error = from_grpc_error(MockGrpcError())
        assert isinstance(error, UnavailableError)
        assert "service unavailable" in str(error)

    def test_not_leader_error_mapping(self):
        """Should map NOT_LEADER to NotLeaderError."""

        class MockGrpcError:
            def code(self):
                class Code:
                    value = [14]

                return Code()

            def details(self):
                return "NOT_LEADER: node is not leader"

        class MockMetadata:
            def get(self, key):
                if key == "leader-hint":
                    return ["localhost:50052"]
                return None

        error = from_grpc_error(MockGrpcError(), MockMetadata())
        assert isinstance(error, NotLeaderError)
        assert error.leader_hint == "localhost:50052"

    def test_deadline_exceeded_mapping(self):
        """Should map gRPC DEADLINE_EXCEEDED."""

        class MockGrpcError:
            def code(self):
                class Code:
                    value = [4]

                return Code()

            def details(self):
                return "deadline exceeded"

        error = from_grpc_error(MockGrpcError())
        assert isinstance(error, DeadlineExceededError)

    def test_invalid_argument_mapping(self):
        """Should map gRPC INVALID_ARGUMENT."""

        class MockGrpcError:
            def code(self):
                class Code:
                    value = [3]

                return Code()

            def details(self):
                return "invalid argument"

        error = from_grpc_error(MockGrpcError())
        assert isinstance(error, InvalidArgumentError)

    def test_already_exists_mapping(self):
        """Should map gRPC ALREADY_EXISTS."""

        class MockGrpcError:
            def code(self):
                class Code:
                    value = [6]

                return Code()

            def details(self):
                return "key already exists"

        error = from_grpc_error(MockGrpcError())
        assert isinstance(error, AlreadyExistsError)

    def test_version_mismatch_mapping(self):
        """Should map gRPC FAILED_PRECONDITION to VersionMismatchError."""

        class MockGrpcError:
            def code(self):
                class Code:
                    value = [9]

                return Code()

            def details(self):
                return "version mismatch"

        error = from_grpc_error(MockGrpcError())
        assert isinstance(error, VersionMismatchError)

    def test_unknown_error_mapping(self):
        """Should map unknown errors to base NoriKVError."""

        class MockGrpcError:
            def code(self):
                class Code:
                    value = [999]

                return Code()

            def details(self):
                return "unknown error"

        error = from_grpc_error(MockGrpcError())
        assert isinstance(error, NoriKVError)
        assert error.code == "999"

    def test_error_without_code(self):
        """Should handle errors without proper code."""

        class MockGrpcError:
            def code(self):
                return None

            def details(self):
                return "error without code"

        error = from_grpc_error(MockGrpcError())
        assert isinstance(error, NoriKVError)
        assert "error without code" in str(error)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
