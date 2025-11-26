package norikv

import (
	"context"

	"github.com/norikv/norikv-go/proto"
	"google.golang.org/grpc"
	"google.golang.org/grpc/metadata"
)

// VectorCreateIndex creates a vector index for the specified namespace.
func (c *Client) VectorCreateIndex(
	ctx context.Context,
	namespace string,
	dimensions uint32,
	distance DistanceFunction,
	indexType VectorIndexType,
	opts *CreateVectorIndexOptions,
) (bool, error) {
	if namespace == "" {
		return false, NewInvalidArgumentError("namespace cannot be empty")
	}

	if dimensions == 0 {
		return false, NewInvalidArgumentError("dimensions must be greater than 0")
	}

	if opts == nil {
		opts = &CreateVectorIndexOptions{}
	}

	ctx, cancel := c.withTimeout(ctx)
	defer cancel()

	var created bool

	err := c.retry.Execute(ctx, func() error {
		// Get any connection (vector operations typically go to a single shard)
		conn, address, err := c.getAnyConnection(ctx)
		if err != nil {
			return err
		}

		client := proto.NewVectorClient(conn)

		// Map local types to proto types
		protoDistance := proto.DistanceFunction_DISTANCE_FUNCTION_UNSPECIFIED
		switch distance {
		case DistanceEuclidean:
			protoDistance = proto.DistanceFunction_DISTANCE_FUNCTION_EUCLIDEAN
		case DistanceCosine:
			protoDistance = proto.DistanceFunction_DISTANCE_FUNCTION_COSINE
		case DistanceInnerProduct:
			protoDistance = proto.DistanceFunction_DISTANCE_FUNCTION_INNER_PRODUCT
		}

		protoIndexType := proto.VectorIndexType_VECTOR_INDEX_TYPE_UNSPECIFIED
		switch indexType {
		case VectorIndexBruteForce:
			protoIndexType = proto.VectorIndexType_VECTOR_INDEX_TYPE_BRUTE_FORCE
		case VectorIndexHNSW:
			protoIndexType = proto.VectorIndexType_VECTOR_INDEX_TYPE_HNSW
		}

		req := &proto.CreateVectorIndexRequest{
			Namespace:      namespace,
			Dimensions:     dimensions,
			Distance:       protoDistance,
			IndexType:      protoIndexType,
			IdempotencyKey: opts.IdempotencyKey,
		}

		var md metadata.MD
		resp, err := client.CreateIndex(ctx, req, grpc.Trailer(&md))
		if err != nil {
			err = FromGRPCError(err, md)
			if _, ok := err.(*NotLeaderError); ok {
				c.pool.Remove(address)
				return err
			}
			return err
		}

		created = resp.Created
		return nil
	})

	return created, err
}

// VectorDropIndex drops a vector index for the specified namespace.
func (c *Client) VectorDropIndex(
	ctx context.Context,
	namespace string,
	opts *DropVectorIndexOptions,
) (bool, error) {
	if namespace == "" {
		return false, NewInvalidArgumentError("namespace cannot be empty")
	}

	if opts == nil {
		opts = &DropVectorIndexOptions{}
	}

	ctx, cancel := c.withTimeout(ctx)
	defer cancel()

	var dropped bool

	err := c.retry.Execute(ctx, func() error {
		conn, address, err := c.getAnyConnection(ctx)
		if err != nil {
			return err
		}

		client := proto.NewVectorClient(conn)

		req := &proto.DropVectorIndexRequest{
			Namespace:      namespace,
			IdempotencyKey: opts.IdempotencyKey,
		}

		var md metadata.MD
		resp, err := client.DropIndex(ctx, req, grpc.Trailer(&md))
		if err != nil {
			err = FromGRPCError(err, md)
			if _, ok := err.(*NotLeaderError); ok {
				c.pool.Remove(address)
				return err
			}
			return err
		}

		dropped = resp.Dropped
		return nil
	})

	return dropped, err
}

// VectorInsert inserts a vector into the specified namespace.
func (c *Client) VectorInsert(
	ctx context.Context,
	namespace string,
	id string,
	vector []float32,
	opts *VectorInsertOptions,
) (*Version, error) {
	if namespace == "" {
		return nil, NewInvalidArgumentError("namespace cannot be empty")
	}

	if id == "" {
		return nil, NewInvalidArgumentError("id cannot be empty")
	}

	if len(vector) == 0 {
		return nil, NewInvalidArgumentError("vector cannot be empty")
	}

	if opts == nil {
		opts = &VectorInsertOptions{}
	}

	ctx, cancel := c.withTimeout(ctx)
	defer cancel()

	var version *Version

	err := c.retry.Execute(ctx, func() error {
		conn, address, err := c.getAnyConnection(ctx)
		if err != nil {
			return err
		}

		client := proto.NewVectorClient(conn)

		req := &proto.VectorInsertRequest{
			Namespace:      namespace,
			Id:             id,
			Vector:         vector,
			IdempotencyKey: opts.IdempotencyKey,
		}

		var md metadata.MD
		resp, err := client.Insert(ctx, req, grpc.Trailer(&md))
		if err != nil {
			err = FromGRPCError(err, md)
			if _, ok := err.(*NotLeaderError); ok {
				c.pool.Remove(address)
				return err
			}
			return err
		}

		v := &Version{}
		version = v.FromProto(resp.Version)
		return nil
	})

	return version, err
}

// VectorDelete deletes a vector from the specified namespace.
func (c *Client) VectorDelete(
	ctx context.Context,
	namespace string,
	id string,
	opts *VectorDeleteOptions,
) (bool, error) {
	if namespace == "" {
		return false, NewInvalidArgumentError("namespace cannot be empty")
	}

	if id == "" {
		return false, NewInvalidArgumentError("id cannot be empty")
	}

	if opts == nil {
		opts = &VectorDeleteOptions{}
	}

	ctx, cancel := c.withTimeout(ctx)
	defer cancel()

	var deleted bool

	err := c.retry.Execute(ctx, func() error {
		conn, address, err := c.getAnyConnection(ctx)
		if err != nil {
			return err
		}

		client := proto.NewVectorClient(conn)

		req := &proto.VectorDeleteRequest{
			Namespace:      namespace,
			Id:             id,
			IdempotencyKey: opts.IdempotencyKey,
		}

		var md metadata.MD
		resp, err := client.Delete(ctx, req, grpc.Trailer(&md))
		if err != nil {
			err = FromGRPCError(err, md)
			if _, ok := err.(*NotLeaderError); ok {
				c.pool.Remove(address)
				return err
			}
			return err
		}

		deleted = resp.Deleted
		return nil
	})

	return deleted, err
}

// VectorSearch searches for nearest neighbors in the specified namespace.
func (c *Client) VectorSearch(
	ctx context.Context,
	namespace string,
	query []float32,
	k uint32,
	opts *VectorSearchOptions,
) (*VectorSearchResult, error) {
	if namespace == "" {
		return nil, NewInvalidArgumentError("namespace cannot be empty")
	}

	if len(query) == 0 {
		return nil, NewInvalidArgumentError("query vector cannot be empty")
	}

	if k == 0 {
		return nil, NewInvalidArgumentError("k must be greater than 0")
	}

	if opts == nil {
		opts = &VectorSearchOptions{}
	}

	ctx, cancel := c.withTimeout(ctx)
	defer cancel()

	var result *VectorSearchResult

	err := c.retry.Execute(ctx, func() error {
		conn, address, err := c.getAnyConnection(ctx)
		if err != nil {
			return err
		}

		client := proto.NewVectorClient(conn)

		req := &proto.VectorSearchRequest{
			Namespace:      namespace,
			Query:          query,
			K:              k,
			IncludeVectors: opts.IncludeVectors,
		}

		var md metadata.MD
		resp, err := client.Search(ctx, req, grpc.Trailer(&md))
		if err != nil {
			err = FromGRPCError(err, md)
			if _, ok := err.(*NotLeaderError); ok {
				c.pool.Remove(address)
				return err
			}
			return err
		}

		matches := make([]VectorMatch, len(resp.Matches))
		for i, m := range resp.Matches {
			matches[i] = VectorMatch{
				ID:       m.Id,
				Distance: m.Distance,
				Vector:   m.Vector,
			}
		}

		result = &VectorSearchResult{
			Matches:      matches,
			SearchTimeUs: resp.SearchTimeUs,
		}
		return nil
	})

	return result, err
}

// VectorGet retrieves a specific vector by ID from the namespace.
func (c *Client) VectorGet(
	ctx context.Context,
	namespace string,
	id string,
) ([]float32, error) {
	if namespace == "" {
		return nil, NewInvalidArgumentError("namespace cannot be empty")
	}

	if id == "" {
		return nil, NewInvalidArgumentError("id cannot be empty")
	}

	ctx, cancel := c.withTimeout(ctx)
	defer cancel()

	var vector []float32

	err := c.retry.Execute(ctx, func() error {
		conn, address, err := c.getAnyConnection(ctx)
		if err != nil {
			return err
		}

		client := proto.NewVectorClient(conn)

		req := &proto.VectorGetRequest{
			Namespace: namespace,
			Id:        id,
		}

		var md metadata.MD
		resp, err := client.Get(ctx, req, grpc.Trailer(&md))
		if err != nil {
			err = FromGRPCError(err, md)
			if _, ok := err.(*NotLeaderError); ok {
				c.pool.Remove(address)
				return err
			}
			return err
		}

		if !resp.Found {
			return NewNotFoundError(id)
		}

		vector = resp.Vector
		return nil
	})

	return vector, err
}

// getAnyConnection returns a connection to any available node.
// Used for operations that don't require key-based routing.
func (c *Client) getAnyConnection(ctx context.Context) (*grpc.ClientConn, string, error) {
	nodes := c.topology.GetNodes()
	if len(nodes) == 0 {
		nodes = c.config.Nodes
	}

	for _, addr := range nodes {
		conn, err := c.pool.Get(ctx, addr)
		if err == nil {
			return conn, addr, nil
		}
	}

	return nil, "", NewUnavailableError("no available nodes")
}
