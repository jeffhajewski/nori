"""Custom error classes for NoriKV client."""

from typing import Any, Optional

from norikv.types import Version


class NoriKVError(Exception):
    """Base error class for all NoriKV errors."""

    def __init__(self, message: str, code: Optional[str] = None):
        super().__init__(message)
        self.code = code

    def __repr__(self) -> str:
        if self.code:
            return f"{self.__class__.__name__}(message={str(self)!r}, code={self.code!r})"
        return f"{self.__class__.__name__}(message={str(self)!r})"


class NotLeaderError(NoriKVError):
    """Error indicating the contacted node is not the leader for the requested shard.

    Client should retry on the leader node indicated in the error.
    """

    def __init__(
        self,
        message: str,
        leader_hint: Optional[str] = None,
        shard_id: Optional[int] = None,
    ):
        super().__init__(message, "NOT_LEADER")
        self.leader_hint = leader_hint
        self.shard_id = shard_id

    def __repr__(self) -> str:
        return (
            f"NotLeaderError(message={str(self)!r}, "
            f"leader_hint={self.leader_hint!r}, shard_id={self.shard_id!r})"
        )


class AlreadyExistsError(NoriKVError):
    """Error indicating the key already exists (for if_not_exists operations)."""

    def __init__(self, message: str, key: bytes):
        super().__init__(message, "ALREADY_EXISTS")
        self.key = key

    def __repr__(self) -> str:
        return f"AlreadyExistsError(message={str(self)!r}, key={self.key!r})"


class VersionMismatchError(NoriKVError):
    """Error indicating version mismatch (for if_match_version operations)."""

    def __init__(
        self,
        message: str,
        key: bytes,
        expected_version: Optional[Version],
        actual_version: Optional[Version],
    ):
        super().__init__(message, "VERSION_MISMATCH")
        self.key = key
        self.expected_version = expected_version
        self.actual_version = actual_version

    def __repr__(self) -> str:
        return (
            f"VersionMismatchError(message={str(self)!r}, "
            f"key={self.key!r}, expected={self.expected_version!r}, "
            f"actual={self.actual_version!r})"
        )


class UnavailableError(NoriKVError):
    """Error indicating the service is temporarily unavailable.

    Client should retry with backoff.
    """

    def __init__(self, message: str):
        super().__init__(message, "UNAVAILABLE")


class DeadlineExceededError(NoriKVError):
    """Error indicating the request exceeded the deadline/timeout."""

    def __init__(self, message: str, timeout_ms: int):
        super().__init__(message, "DEADLINE_EXCEEDED")
        self.timeout_ms = timeout_ms

    def __repr__(self) -> str:
        return (
            f"DeadlineExceededError(message={str(self)!r}, "
            f"timeout_ms={self.timeout_ms})"
        )


class InvalidArgumentError(NoriKVError):
    """Error indicating invalid request parameters."""

    def __init__(self, message: str):
        super().__init__(message, "INVALID_ARGUMENT")


class ConnectionError(NoriKVError):
    """Error indicating connection failure to the cluster."""

    def __init__(self, message: str, address: Optional[str] = None):
        super().__init__(message, "CONNECTION_ERROR")
        self.address = address

    def __repr__(self) -> str:
        return (
            f"ConnectionError(message={str(self)!r}, address={self.address!r})"
        )


class NoNodesAvailableError(NoriKVError):
    """Error indicating no nodes are available."""

    def __init__(self, message: str = "No nodes available in the cluster"):
        super().__init__(message, "NO_NODES_AVAILABLE")


class RetryExhaustedError(NoriKVError):
    """Error indicating maximum retry attempts have been exhausted."""

    def __init__(
        self,
        message: str,
        attempts: int,
        last_error: Optional[Exception] = None,
    ):
        super().__init__(message, "RETRY_EXHAUSTED")
        self.attempts = attempts
        self.last_error = last_error

    def __repr__(self) -> str:
        return (
            f"RetryExhaustedError(message={str(self)!r}, "
            f"attempts={self.attempts}, last_error={self.last_error!r})"
        )


class NotFoundError(NoriKVError):
    """Error indicating the requested key or resource was not found."""

    def __init__(self, message: str, id: Optional[str] = None):
        super().__init__(message, "NOT_FOUND")
        self.id = id

    def __repr__(self) -> str:
        return f"NotFoundError(message={str(self)!r}, id={self.id!r})"


def from_grpc_error(error: Any, metadata: Optional[Any] = None) -> NoriKVError:
    """Convert gRPC status code to NoriKV error.

    Args:
        error: gRPC error with code and details
        metadata: Optional gRPC metadata containing leader hints

    Returns:
        Appropriate NoriKVError subclass
    """
    try:
        # Try to get gRPC status code
        code = getattr(error, "code", lambda: None)()
        if code is not None:
            code = code.value[0] if hasattr(code, "value") else code
    except Exception:
        code = None

    message = (
        getattr(error, "details", lambda: None)()
        or str(error)
        or "Unknown error"
    )

    # Extract leader hint from metadata if present
    leader_hint = None
    if metadata is not None:
        try:
            if hasattr(metadata, "get"):
                leader_hint_list = metadata.get("leader-hint")
                if leader_hint_list:
                    leader_hint = leader_hint_list[0]
        except Exception:
            pass

    # Map gRPC status codes to NoriKV errors
    # See: https://grpc.github.io/grpc/core/md_doc_statuscodes.html
    if code == 14:  # UNAVAILABLE
        if "NOT_LEADER" in message:
            return NotLeaderError(message, leader_hint)
        return UnavailableError(message)

    if code == 4:  # DEADLINE_EXCEEDED
        return DeadlineExceededError(message, 0)

    if code == 3:  # INVALID_ARGUMENT
        return InvalidArgumentError(message)

    if code == 6:  # ALREADY_EXISTS
        return AlreadyExistsError(message, b"")

    if code == 9:  # FAILED_PRECONDITION (version mismatch)
        return VersionMismatchError(message, b"", None, None)

    if code == 5:  # NOT_FOUND
        return NotFoundError(message)

    # Default to base NoriKVError
    return NoriKVError(message, str(code) if code is not None else None)
