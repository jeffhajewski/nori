"""Generated Protocol Buffer code for NoriKV.

This package contains auto-generated code from norikv.proto.
Do not edit these files manually.
"""

from norikv.proto.norikv_pb2 import (
    ClusterNode,
    ClusterView,
    DeleteRequest,
    DeleteResponse,
    GetRequest,
    GetResponse,
    PutRequest,
    PutResponse,
    ShardInfo,
    ShardReplica,
    Version,
)
from norikv.proto.norikv_pb2_grpc import (
    AdminStub,
    KvStub,
    MetaStub,
)

__all__ = [
    # Messages
    "Version",
    "PutRequest",
    "PutResponse",
    "GetRequest",
    "GetResponse",
    "DeleteRequest",
    "DeleteResponse",
    "ClusterNode",
    "ShardReplica",
    "ShardInfo",
    "ClusterView",
    # Services
    "KvStub",
    "MetaStub",
    "AdminStub",
]
