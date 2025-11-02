// GENERATED CODE -- DO NOT EDIT!

'use strict';
var grpc = require('@grpc/grpc-js');
var norikv_pb = require('./norikv_pb.js');

function serialize_norikv_v1_ClusterView(arg) {
  if (!(arg instanceof norikv_pb.ClusterView)) {
    throw new Error('Expected argument of type norikv.v1.ClusterView');
  }
  return Buffer.from(arg.serializeBinary());
}

function deserialize_norikv_v1_ClusterView(buffer_arg) {
  return norikv_pb.ClusterView.deserializeBinary(new Uint8Array(buffer_arg));
}

function serialize_norikv_v1_DeleteRequest(arg) {
  if (!(arg instanceof norikv_pb.DeleteRequest)) {
    throw new Error('Expected argument of type norikv.v1.DeleteRequest');
  }
  return Buffer.from(arg.serializeBinary());
}

function deserialize_norikv_v1_DeleteRequest(buffer_arg) {
  return norikv_pb.DeleteRequest.deserializeBinary(new Uint8Array(buffer_arg));
}

function serialize_norikv_v1_DeleteResponse(arg) {
  if (!(arg instanceof norikv_pb.DeleteResponse)) {
    throw new Error('Expected argument of type norikv.v1.DeleteResponse');
  }
  return Buffer.from(arg.serializeBinary());
}

function deserialize_norikv_v1_DeleteResponse(buffer_arg) {
  return norikv_pb.DeleteResponse.deserializeBinary(new Uint8Array(buffer_arg));
}

function serialize_norikv_v1_GetRequest(arg) {
  if (!(arg instanceof norikv_pb.GetRequest)) {
    throw new Error('Expected argument of type norikv.v1.GetRequest');
  }
  return Buffer.from(arg.serializeBinary());
}

function deserialize_norikv_v1_GetRequest(buffer_arg) {
  return norikv_pb.GetRequest.deserializeBinary(new Uint8Array(buffer_arg));
}

function serialize_norikv_v1_GetResponse(arg) {
  if (!(arg instanceof norikv_pb.GetResponse)) {
    throw new Error('Expected argument of type norikv.v1.GetResponse');
  }
  return Buffer.from(arg.serializeBinary());
}

function deserialize_norikv_v1_GetResponse(buffer_arg) {
  return norikv_pb.GetResponse.deserializeBinary(new Uint8Array(buffer_arg));
}

function serialize_norikv_v1_PutRequest(arg) {
  if (!(arg instanceof norikv_pb.PutRequest)) {
    throw new Error('Expected argument of type norikv.v1.PutRequest');
  }
  return Buffer.from(arg.serializeBinary());
}

function deserialize_norikv_v1_PutRequest(buffer_arg) {
  return norikv_pb.PutRequest.deserializeBinary(new Uint8Array(buffer_arg));
}

function serialize_norikv_v1_PutResponse(arg) {
  if (!(arg instanceof norikv_pb.PutResponse)) {
    throw new Error('Expected argument of type norikv.v1.PutResponse');
  }
  return Buffer.from(arg.serializeBinary());
}

function deserialize_norikv_v1_PutResponse(buffer_arg) {
  return norikv_pb.PutResponse.deserializeBinary(new Uint8Array(buffer_arg));
}

function serialize_norikv_v1_ShardInfo(arg) {
  if (!(arg instanceof norikv_pb.ShardInfo)) {
    throw new Error('Expected argument of type norikv.v1.ShardInfo');
  }
  return Buffer.from(arg.serializeBinary());
}

function deserialize_norikv_v1_ShardInfo(buffer_arg) {
  return norikv_pb.ShardInfo.deserializeBinary(new Uint8Array(buffer_arg));
}


var KvService = exports.KvService = {
  put: {
    path: '/norikv.v1.Kv/Put',
    requestStream: false,
    responseStream: false,
    requestType: norikv_pb.PutRequest,
    responseType: norikv_pb.PutResponse,
    requestSerialize: serialize_norikv_v1_PutRequest,
    requestDeserialize: deserialize_norikv_v1_PutRequest,
    responseSerialize: serialize_norikv_v1_PutResponse,
    responseDeserialize: deserialize_norikv_v1_PutResponse,
  },
  get: {
    path: '/norikv.v1.Kv/Get',
    requestStream: false,
    responseStream: false,
    requestType: norikv_pb.GetRequest,
    responseType: norikv_pb.GetResponse,
    requestSerialize: serialize_norikv_v1_GetRequest,
    requestDeserialize: deserialize_norikv_v1_GetRequest,
    responseSerialize: serialize_norikv_v1_GetResponse,
    responseDeserialize: deserialize_norikv_v1_GetResponse,
  },
  delete: {
    path: '/norikv.v1.Kv/Delete',
    requestStream: false,
    responseStream: false,
    requestType: norikv_pb.DeleteRequest,
    responseType: norikv_pb.DeleteResponse,
    requestSerialize: serialize_norikv_v1_DeleteRequest,
    requestDeserialize: deserialize_norikv_v1_DeleteRequest,
    responseSerialize: serialize_norikv_v1_DeleteResponse,
    responseDeserialize: deserialize_norikv_v1_DeleteResponse,
  },
};

exports.KvClient = grpc.makeGenericClientConstructor(KvService, 'Kv');
var MetaService = exports.MetaService = {
  watchCluster: {
    path: '/norikv.v1.Meta/WatchCluster',
    requestStream: false,
    responseStream: true,
    requestType: norikv_pb.ClusterView,
    responseType: norikv_pb.ClusterView,
    requestSerialize: serialize_norikv_v1_ClusterView,
    requestDeserialize: deserialize_norikv_v1_ClusterView,
    responseSerialize: serialize_norikv_v1_ClusterView,
    responseDeserialize: deserialize_norikv_v1_ClusterView,
  },
};

exports.MetaClient = grpc.makeGenericClientConstructor(MetaService, 'Meta');
var AdminService = exports.AdminService = {
  transferShard: {
    path: '/norikv.v1.Admin/TransferShard',
    requestStream: false,
    responseStream: false,
    requestType: norikv_pb.ShardInfo,
    responseType: norikv_pb.ShardInfo,
    requestSerialize: serialize_norikv_v1_ShardInfo,
    requestDeserialize: deserialize_norikv_v1_ShardInfo,
    responseSerialize: serialize_norikv_v1_ShardInfo,
    responseDeserialize: deserialize_norikv_v1_ShardInfo,
  },
  snapshotShard: {
    path: '/norikv.v1.Admin/SnapshotShard',
    requestStream: false,
    responseStream: false,
    requestType: norikv_pb.ShardInfo,
    responseType: norikv_pb.ShardInfo,
    requestSerialize: serialize_norikv_v1_ShardInfo,
    requestDeserialize: deserialize_norikv_v1_ShardInfo,
    responseSerialize: serialize_norikv_v1_ShardInfo,
    responseDeserialize: deserialize_norikv_v1_ShardInfo,
  },
};

exports.AdminClient = grpc.makeGenericClientConstructor(AdminService, 'Admin');
