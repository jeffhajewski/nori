"""Ephemeral (in-memory) NoriKV instance for testing and development.

This module provides utilities for spawning a temporary NoriKV server process
configured for single-node, ephemeral storage. The server automatically cleans
up when stopped.

Example:
    >>> import asyncio
    >>> from norikv import create_ephemeral
    >>>
    >>> async def main():
    ...     async with create_ephemeral() as cluster:
    ...         client = cluster.get_client()
    ...         await client.put("key", "value")
    ...         result = await client.get("key")
    ...         print(result.value)
    >>>
    >>> asyncio.run(main())
"""

import asyncio
import os
import shutil
import socket
import subprocess
import tempfile
import time
import uuid
import yaml
from pathlib import Path
from typing import Optional
from contextlib import asynccontextmanager

from norikv.client import NoriKVClient
from norikv.config import ClientConfig
from norikv.errors import NoriKVError


class EphemeralServerError(NoriKVError):
    """Error related to ephemeral server management."""
    pass


def _find_free_port() -> int:
    """Find an available port on localhost."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(('127.0.0.1', 0))
        s.listen(1)
        port = s.getsockname()[1]
    return port


def _find_norikv_server_binary() -> str:
    """Find the norikv-server binary.

    Searches in the following order:
    1. NORIKV_SERVER_PATH environment variable
    2. Same directory as this Python package
    3. System PATH

    Returns:
        Path to the norikv-server binary

    Raises:
        EphemeralServerError: If binary not found
    """
    # Check environment variable
    env_path = os.environ.get('NORIKV_SERVER_PATH')
    if env_path and os.path.isfile(env_path) and os.access(env_path, os.X_OK):
        return env_path

    # Check package directory (for bundled binaries)
    package_dir = Path(__file__).parent
    bundled_binary = package_dir / 'bin' / 'norikv-server'
    if bundled_binary.exists() and os.access(bundled_binary, os.X_OK):
        return str(bundled_binary)

    # Check system PATH
    server_path = shutil.which('norikv-server')
    if server_path:
        return server_path

    # Not found - raise helpful error
    raise EphemeralServerError(
        "norikv-server binary not found. Please install it or set NORIKV_SERVER_PATH.\n"
        "\n"
        "Installation options:\n"
        "  1. Build from source: cargo build --release -p norikv-server\n"
        "  2. Set NORIKV_SERVER_PATH to point to the binary\n"
        "  3. Add the binary to your system PATH"
    )


class EphemeralNoriKV:
    """Manages an ephemeral NoriKV server instance.

    This class spawns a local norikv-server process configured for single-node,
    ephemeral storage. The server runs on an automatically assigned port and
    stores data in a temporary directory that is cleaned up on exit.

    Args:
        port: Optional port number. If None, an available port is auto-assigned.
        data_dir: Optional data directory. If None, a temp directory is created.
        total_shards: Number of virtual shards (default: 256 for faster startup).
        server_binary: Optional path to norikv-server binary.
        log_file: Optional path to log file. If None, logs go to temp file.
        startup_timeout: Seconds to wait for server to become ready (default: 10).

    Example:
        >>> async def example():
        ...     cluster = EphemeralNoriKV()
        ...     await cluster.start()
        ...     client = cluster.get_client()
        ...     await client.put("key", "value")
        ...     await cluster.stop()
    """

    def __init__(
        self,
        port: Optional[int] = None,
        data_dir: Optional[str] = None,
        total_shards: int = 256,
        server_binary: Optional[str] = None,
        log_file: Optional[str] = None,
        startup_timeout: float = 10.0,
    ):
        """Initialize ephemeral server configuration."""
        self.port = port or _find_free_port()
        self.data_dir = data_dir
        self.total_shards = total_shards
        self.server_binary = server_binary
        self.log_file = log_file
        self.startup_timeout = startup_timeout

        # Runtime state
        self._process: Optional[subprocess.Popen] = None
        self._temp_dir: Optional[str] = None
        self._temp_log: Optional[str] = None
        self._config_file: Optional[str] = None
        self._client: Optional[NoriKVClient] = None
        self._started = False

    async def start(self) -> None:
        """Start the ephemeral server.

        This method:
        1. Creates a temporary directory for data
        2. Generates server configuration
        3. Spawns the norikv-server process
        4. Waits for the server to become healthy

        Raises:
            EphemeralServerError: If server fails to start or become healthy
        """
        if self._started:
            raise EphemeralServerError("Server already started")

        # Find server binary
        binary = self.server_binary or _find_norikv_server_binary()

        # Create temp directory if needed
        if not self.data_dir:
            self._temp_dir = tempfile.mkdtemp(prefix='norikv-ephemeral-')
            actual_data_dir = self._temp_dir
        else:
            actual_data_dir = self.data_dir
            os.makedirs(actual_data_dir, exist_ok=True)

        # Create temp log file if needed
        if not self.log_file:
            log_fd, self._temp_log = tempfile.mkstemp(
                prefix='norikv-ephemeral-',
                suffix='.log'
            )
            os.close(log_fd)
            actual_log_file = self._temp_log
        else:
            actual_log_file = self.log_file

        # Generate node configuration
        node_id = f"ephemeral-{uuid.uuid4().hex[:8]}"
        config = {
            'node_id': node_id,
            'rpc_addr': f'127.0.0.1:{self.port}',
            'data_dir': actual_data_dir,
            'cluster': {
                'seed_nodes': [],  # Single node, no cluster
                'total_shards': self.total_shards,
                'replication_factor': 1,
            },
            'telemetry': {
                'prometheus': {
                    'enabled': False,  # Disable metrics for ephemeral mode
                },
                'otlp': {
                    'enabled': False,
                }
            }
        }

        # Write config to temp file
        config_fd, self._config_file = tempfile.mkstemp(
            prefix='norikv-config-',
            suffix='.yaml'
        )
        with os.fdopen(config_fd, 'w') as f:
            yaml.dump(config, f)

        # Spawn server process
        try:
            with open(actual_log_file, 'w') as log_f:
                self._process = subprocess.Popen(
                    [binary, self._config_file],
                    stdout=log_f,
                    stderr=subprocess.STDOUT,
                    env={**os.environ, 'RUST_LOG': 'info'},
                )
        except Exception as e:
            self._cleanup()
            raise EphemeralServerError(f"Failed to start server: {e}") from e

        # Wait for server to become healthy
        await self._wait_for_health()

        self._started = True

    async def _wait_for_health(self) -> None:
        """Wait for server to become healthy.

        Polls the server by attempting to connect via gRPC until it responds
        or the timeout is reached.

        Raises:
            EphemeralServerError: If server doesn't become healthy in time
        """
        start_time = time.time()
        last_error = None

        while time.time() - start_time < self.startup_timeout:
            # Check if process has died
            if self._process.poll() is not None:
                raise EphemeralServerError(
                    f"Server process exited with code {self._process.returncode}. "
                    f"Check logs at: {self.log_file or self._temp_log}"
                )

            # Try to connect
            try:
                # Create a temporary client to test connectivity
                test_client = NoriKVClient(ClientConfig(
                    nodes=[f'localhost:{self.port}'],
                    total_shards=self.total_shards,
                ))

                async with test_client:
                    # Try a simple operation to verify server is working
                    # We just need to connect successfully
                    pass

                # Success!
                return

            except Exception as e:
                last_error = e
                await asyncio.sleep(0.1)  # Wait before retry

        # Timeout reached
        self._cleanup()
        raise EphemeralServerError(
            f"Server did not become healthy within {self.startup_timeout}s. "
            f"Last error: {last_error}. "
            f"Check logs at: {self.log_file or self._temp_log}"
        )

    def get_client(self) -> NoriKVClient:
        """Get a client configured to connect to this ephemeral server.

        Returns:
            NoriKVClient instance configured for this server

        Raises:
            EphemeralServerError: If server is not started
        """
        if not self._started:
            raise EphemeralServerError(
                "Server not started. Call start() first or use as context manager."
            )

        if not self._client:
            self._client = NoriKVClient(ClientConfig(
                nodes=[f'localhost:{self.port}'],
                total_shards=self.total_shards,
            ))

        return self._client

    async def stop(self) -> None:
        """Stop the ephemeral server and clean up resources.

        This method:
        1. Sends SIGTERM to the server process
        2. Waits up to 5 seconds for graceful shutdown
        3. Sends SIGKILL if necessary
        4. Cleans up temporary files and directories
        """
        if not self._started:
            return

        # Close client if we created one
        if self._client:
            try:
                await self._client.__aexit__(None, None, None)
            except Exception:
                pass  # Ignore errors during client shutdown
            self._client = None

        # Stop server process
        if self._process:
            try:
                self._process.terminate()

                # Wait up to 5 seconds for graceful shutdown
                try:
                    self._process.wait(timeout=5.0)
                except subprocess.TimeoutExpired:
                    # Force kill if not terminated
                    self._process.kill()
                    self._process.wait()
            except Exception:
                pass  # Ignore errors during process cleanup

            self._process = None

        # Clean up temp resources
        self._cleanup()
        self._started = False

    def _cleanup(self) -> None:
        """Clean up temporary files and directories."""
        if self._config_file and os.path.exists(self._config_file):
            try:
                os.remove(self._config_file)
            except Exception:
                pass
            self._config_file = None

        if self._temp_log and os.path.exists(self._temp_log):
            try:
                os.remove(self._temp_log)
            except Exception:
                pass
            self._temp_log = None

        if self._temp_dir and os.path.exists(self._temp_dir):
            try:
                shutil.rmtree(self._temp_dir)
            except Exception:
                pass
            self._temp_dir = None

    async def __aenter__(self) -> 'EphemeralNoriKV':
        """Context manager entry - starts the server."""
        await self.start()
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb) -> None:
        """Context manager exit - stops the server."""
        await self.stop()


@asynccontextmanager
async def create_ephemeral(
    port: Optional[int] = None,
    data_dir: Optional[str] = None,
    total_shards: int = 256,
    **kwargs
) -> EphemeralNoriKV:
    """Create and start an ephemeral NoriKV server (async context manager).

    This is a convenience function that creates an EphemeralNoriKV instance,
    starts it, and ensures cleanup on exit.

    Args:
        port: Optional port number. If None, an available port is auto-assigned.
        data_dir: Optional data directory. If None, a temp directory is created.
        total_shards: Number of virtual shards (default: 256).
        **kwargs: Additional arguments passed to EphemeralNoriKV constructor.

    Yields:
        EphemeralNoriKV instance that is started and ready to use

    Example:
        >>> async def example():
        ...     async with create_ephemeral() as cluster:
        ...         client = cluster.get_client()
        ...         async with client:
        ...             await client.put("key", "value")
        ...             result = await client.get("key")
        ...             print(result.value.decode())
    """
    cluster = EphemeralNoriKV(
        port=port,
        data_dir=data_dir,
        total_shards=total_shards,
        **kwargs
    )

    try:
        await cluster.start()
        yield cluster
    finally:
        await cluster.stop()
