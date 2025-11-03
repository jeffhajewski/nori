"""Retry logic with exponential backoff for transient failures."""

import asyncio
import random
from typing import Callable, TypeVar, Optional, Set, Type

from norikv.errors import (
    NoriKVError,
    UnavailableError,
    DeadlineExceededError,
    NotLeaderError,
    RetryExhaustedError,
)
from norikv.types import RetryConfig

T = TypeVar("T")


class RetryPolicy:
    """Implements retry logic with exponential backoff and jitter."""

    # Errors that are retryable
    RETRYABLE_ERRORS: Set[Type[NoriKVError]] = {
        UnavailableError,
        DeadlineExceededError,
        NotLeaderError,
    }

    def __init__(self, config: RetryConfig):
        """Initialize the retry policy.

        Args:
            config: Retry configuration
        """
        self._config = config

    async def execute_with_retry(
        self,
        operation: Callable[[], T],
        operation_name: str = "operation",
    ) -> T:
        """Execute an operation with retry logic.

        Args:
            operation: Async function to execute
            operation_name: Name of the operation for error messages

        Returns:
            The result of the operation

        Raises:
            RetryExhaustedError: If all retry attempts are exhausted
            NoriKVError: If a non-retryable error occurs
        """
        attempt = 0
        last_error: Optional[Exception] = None

        while attempt < self._config.max_attempts:
            try:
                # Execute the operation
                result = await operation()
                return result

            except Exception as e:
                last_error = e
                attempt += 1

                # Check if error is retryable
                if not self._is_retryable(e):
                    raise

                # Check if we've exhausted retries
                if attempt >= self._config.max_attempts:
                    raise RetryExhaustedError(
                        f"{operation_name} failed after {attempt} attempts",
                        attempts=attempt,
                        last_error=str(e),
                    )

                # Calculate backoff delay
                delay = self._calculate_backoff(attempt)

                # Wait before retry
                await asyncio.sleep(delay)

        # Should not reach here, but handle it anyway
        raise RetryExhaustedError(
            f"{operation_name} failed after {attempt} attempts",
            attempts=attempt,
            last_error=str(last_error) if last_error else "Unknown error",
        )

    def _is_retryable(self, error: Exception) -> bool:
        """Check if an error is retryable.

        Args:
            error: The error to check

        Returns:
            True if the error should be retried
        """
        # Check if error is in retryable set
        for retryable_type in self.RETRYABLE_ERRORS:
            if isinstance(error, retryable_type):
                return True

        return False

    def _calculate_backoff(self, attempt: int) -> float:
        """Calculate backoff delay with exponential backoff and jitter.

        Args:
            attempt: The attempt number (1-indexed)

        Returns:
            Delay in seconds
        """
        # Base delay with exponential backoff
        base_delay = self._config.initial_delay_ms / 1000.0
        delay = base_delay * (self._config.backoff_multiplier ** (attempt - 1))

        # Cap at max delay
        max_delay = self._config.max_delay_ms / 1000.0
        delay = min(delay, max_delay)

        # Add jitter (random factor between 0.5 and 1.5)
        if self._config.jitter:
            jitter_factor = 0.5 + random.random()  # 0.5 to 1.5
            delay *= jitter_factor

        return delay


class CircuitBreaker:
    """Simple circuit breaker to prevent cascading failures."""

    def __init__(
        self,
        failure_threshold: int = 5,
        recovery_timeout: float = 60.0,
        half_open_max_requests: int = 3,
    ):
        """Initialize the circuit breaker.

        Args:
            failure_threshold: Number of failures before opening circuit
            recovery_timeout: Seconds to wait before trying to recover
            half_open_max_requests: Max requests to allow in half-open state
        """
        self._failure_threshold = failure_threshold
        self._recovery_timeout = recovery_timeout
        self._half_open_max_requests = half_open_max_requests

        self._state = "closed"  # closed, open, half-open
        self._failure_count = 0
        self._last_failure_time: Optional[float] = None
        self._half_open_requests = 0
        self._lock = asyncio.Lock()

    async def execute(
        self,
        operation: Callable[[], T],
        fallback: Optional[Callable[[], T]] = None,
    ) -> T:
        """Execute an operation through the circuit breaker.

        Args:
            operation: Async function to execute
            fallback: Optional fallback function if circuit is open

        Returns:
            The result of the operation or fallback

        Raises:
            UnavailableError: If circuit is open and no fallback provided
        """
        async with self._lock:
            # Check circuit state
            if self._state == "open":
                # Check if we should transition to half-open
                if self._should_attempt_recovery():
                    self._state = "half-open"
                    self._half_open_requests = 0
                else:
                    # Circuit is open
                    if fallback:
                        return await fallback()
                    raise UnavailableError(
                        "Circuit breaker is open",
                        retry_after_ms=int(self._recovery_timeout * 1000),
                    )

            # Allow request if closed or half-open (within limit)
            if self._state == "half-open":
                if self._half_open_requests >= self._half_open_max_requests:
                    if fallback:
                        return await fallback()
                    raise UnavailableError(
                        "Circuit breaker is half-open (max requests reached)",
                        retry_after_ms=1000,
                    )
                self._half_open_requests += 1

        # Execute operation outside the lock
        try:
            result = await operation()

            # Success - update state
            async with self._lock:
                if self._state == "half-open":
                    self._state = "closed"
                self._failure_count = 0
                self._last_failure_time = None

            return result

        except Exception as e:
            # Failure - update state
            async with self._lock:
                self._failure_count += 1
                self._last_failure_time = asyncio.get_event_loop().time()

                if self._failure_count >= self._failure_threshold:
                    self._state = "open"

            raise

    def _should_attempt_recovery(self) -> bool:
        """Check if enough time has passed to attempt recovery.

        Returns:
            True if we should try to recover
        """
        if self._last_failure_time is None:
            return True

        current_time = asyncio.get_event_loop().time()
        time_since_failure = current_time - self._last_failure_time

        return time_since_failure >= self._recovery_timeout

    def get_state(self) -> str:
        """Get the current circuit breaker state.

        Returns:
            Current state: "closed", "open", or "half-open"
        """
        return self._state

    def reset(self) -> None:
        """Reset the circuit breaker to closed state."""
        self._state = "closed"
        self._failure_count = 0
        self._last_failure_time = None
        self._half_open_requests = 0
