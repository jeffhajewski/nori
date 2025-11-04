# NoriKV Protocol Buffers

This directory contains the shared Protocol Buffer definitions for NoriKV used by all client SDKs and the server.

## Overview

The proto file defines the gRPC service interface and message types for the NoriKV key-value store.

**File:** `norikv.proto`

## Usage by SDKs

All SDKs reference this single source of truth to ensure protocol compatibility:

### Go SDK
Generate code using:
```bash
cd sdks/go/proto
go generate
```

The `gen.go` file contains a `go:generate` directive that references `../../../proto/norikv.proto`.

### TypeScript SDK
Generate code using:
```bash
cd sdks/typescript
npm run proto:gen
```

The `buf.yaml` file references `../../proto` as the module path.

### Python SDK
Generate code using:
```bash
cd sdks/python
make proto
```

The Makefile uses `python -m grpc_tools.protoc -I../../proto` to reference the shared proto.

### Java SDK
Generate code using:
```bash
cd sdks/java
mvn protobuf:compile protobuf:compile-custom
```

The `pom.xml` configures `<protoSourceRoot>${project.basedir}/../../proto</protoSourceRoot>`.

## Making Changes

When modifying the proto file:

1. Edit `proto/norikv.proto`
2. Regenerate code for all SDKs:
   ```bash
   # Go
   cd sdks/go/proto && go generate

   # TypeScript
   cd sdks/typescript && npm run proto:gen

   # Python
   cd sdks/python && make proto

   # Java
   cd sdks/java && mvn protobuf:compile protobuf:compile-custom
   ```
3. Run tests for all SDKs to verify compatibility
4. Update documentation if the API changed

## Protocol Version

Current version: **v1**

The protocol is versioned through the package name in the proto file. Breaking changes should increment the version number.

## See Also

- [Go SDK](../sdks/go/)
- [TypeScript SDK](../sdks/typescript/)
- [Python SDK](../sdks/python/)
- [Java SDK](../sdks/java/)
