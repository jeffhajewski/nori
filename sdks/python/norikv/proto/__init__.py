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
    # Vector types
    DistanceFunction,
    VectorIndexType,
    CreateVectorIndexRequest,
    CreateVectorIndexResponse,
    DropVectorIndexRequest,
    DropVectorIndexResponse,
    VectorInsertRequest,
    VectorInsertResponse,
    VectorDeleteRequest,
    VectorDeleteResponse,
    VectorSearchRequest,
    VectorSearchResponse,
    VectorGetRequest,
    VectorGetResponse,
    VectorMatch,
)
from norikv.proto.norikv_pb2_grpc import (
    AdminStub,
    KvStub,
    MetaStub,
    VectorStub,
    KvServicer,
    MetaServicer,
    AdminServicer,
    VectorServicer,
    add_KvServicer_to_server,
    add_MetaServicer_to_server,
    add_AdminServicer_to_server,
    add_VectorServicer_to_server,
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
    # Vector messages
    "DistanceFunction",
    "VectorIndexType",
    "CreateVectorIndexRequest",
    "CreateVectorIndexResponse",
    "DropVectorIndexRequest",
    "DropVectorIndexResponse",
    "VectorInsertRequest",
    "VectorInsertResponse",
    "VectorDeleteRequest",
    "VectorDeleteResponse",
    "VectorSearchRequest",
    "VectorSearchResponse",
    "VectorGetRequest",
    "VectorGetResponse",
    "VectorMatch",
    # Service stubs
    "KvStub",
    "MetaStub",
    "AdminStub",
    "VectorStub",
    # Servicers
    "KvServicer",
    "MetaServicer",
    "AdminServicer",
    "VectorServicer",
    # Server functions
    "add_KvServicer_to_server",
    "add_MetaServicer_to_server",
    "add_AdminServicer_to_server",
    "add_VectorServicer_to_server",
]
