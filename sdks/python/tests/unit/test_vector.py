"""Unit tests for vector types and conversions."""

import pytest

from norikv.types import (
    DistanceFunction,
    VectorIndexType,
    VectorMatch,
    VectorSearchResult,
    CreateVectorIndexOptions,
    DropVectorIndexOptions,
    VectorInsertOptions,
    VectorDeleteOptions,
    VectorSearchOptions,
)
from norikv.errors import NotFoundError
from norikv.proto import (
    DistanceFunction as ProtoDistanceFunction,
    VectorIndexType as ProtoVectorIndexType,
)


class TestDistanceFunction:
    """Tests for DistanceFunction enum."""

    def test_euclidean_value(self):
        """Test EUCLIDEAN enum value."""
        assert DistanceFunction.EUCLIDEAN == "euclidean"
        assert DistanceFunction.EUCLIDEAN.value == "euclidean"

    def test_cosine_value(self):
        """Test COSINE enum value."""
        assert DistanceFunction.COSINE == "cosine"
        assert DistanceFunction.COSINE.value == "cosine"

    def test_inner_product_value(self):
        """Test INNER_PRODUCT enum value."""
        assert DistanceFunction.INNER_PRODUCT == "inner_product"
        assert DistanceFunction.INNER_PRODUCT.value == "inner_product"

    def test_from_string(self):
        """Test creating enum from string value."""
        assert DistanceFunction("euclidean") == DistanceFunction.EUCLIDEAN
        assert DistanceFunction("cosine") == DistanceFunction.COSINE
        assert DistanceFunction("inner_product") == DistanceFunction.INNER_PRODUCT


class TestVectorIndexType:
    """Tests for VectorIndexType enum."""

    def test_brute_force_value(self):
        """Test BRUTE_FORCE enum value."""
        assert VectorIndexType.BRUTE_FORCE == "brute_force"
        assert VectorIndexType.BRUTE_FORCE.value == "brute_force"

    def test_hnsw_value(self):
        """Test HNSW enum value."""
        assert VectorIndexType.HNSW == "hnsw"
        assert VectorIndexType.HNSW.value == "hnsw"

    def test_from_string(self):
        """Test creating enum from string value."""
        assert VectorIndexType("brute_force") == VectorIndexType.BRUTE_FORCE
        assert VectorIndexType("hnsw") == VectorIndexType.HNSW


class TestVectorMatch:
    """Tests for VectorMatch dataclass."""

    def test_basic_match(self):
        """Test creating a basic VectorMatch."""
        match = VectorMatch(id="vec-1", distance=0.5)
        assert match.id == "vec-1"
        assert match.distance == 0.5
        assert match.vector is None

    def test_match_with_vector(self):
        """Test VectorMatch with vector data."""
        vector = [0.1, 0.2, 0.3]
        match = VectorMatch(id="vec-2", distance=0.25, vector=vector)
        assert match.id == "vec-2"
        assert match.distance == 0.25
        assert match.vector == [0.1, 0.2, 0.3]


class TestVectorSearchResult:
    """Tests for VectorSearchResult dataclass."""

    def test_empty_result(self):
        """Test empty search result."""
        result = VectorSearchResult(matches=[], search_time_us=100)
        assert len(result.matches) == 0
        assert result.search_time_us == 100

    def test_result_with_matches(self):
        """Test search result with matches."""
        matches = [
            VectorMatch(id="vec-1", distance=0.1),
            VectorMatch(id="vec-2", distance=0.2),
            VectorMatch(id="vec-3", distance=0.3),
        ]
        result = VectorSearchResult(matches=matches, search_time_us=500)
        assert len(result.matches) == 3
        assert result.matches[0].id == "vec-1"
        assert result.matches[0].distance == 0.1
        assert result.search_time_us == 500


class TestVectorOptions:
    """Tests for vector option dataclasses."""

    def test_create_index_options_default(self):
        """Test CreateVectorIndexOptions default values."""
        opts = CreateVectorIndexOptions()
        assert opts.idempotency_key is None

    def test_create_index_options_with_key(self):
        """Test CreateVectorIndexOptions with idempotency key."""
        opts = CreateVectorIndexOptions(idempotency_key="create-123")
        assert opts.idempotency_key == "create-123"

    def test_drop_index_options_default(self):
        """Test DropVectorIndexOptions default values."""
        opts = DropVectorIndexOptions()
        assert opts.idempotency_key is None

    def test_vector_insert_options_default(self):
        """Test VectorInsertOptions default values."""
        opts = VectorInsertOptions()
        assert opts.idempotency_key is None

    def test_vector_delete_options_default(self):
        """Test VectorDeleteOptions default values."""
        opts = VectorDeleteOptions()
        assert opts.idempotency_key is None

    def test_vector_search_options_default(self):
        """Test VectorSearchOptions default values."""
        opts = VectorSearchOptions()
        assert opts.include_vectors is False

    def test_vector_search_options_include_vectors(self):
        """Test VectorSearchOptions with include_vectors."""
        opts = VectorSearchOptions(include_vectors=True)
        assert opts.include_vectors is True


class TestNotFoundError:
    """Tests for NotFoundError class."""

    def test_basic_error(self):
        """Test creating a basic NotFoundError."""
        error = NotFoundError("Vector not found")
        assert str(error) == "Vector not found"
        assert error.code == "NOT_FOUND"
        assert error.id is None

    def test_error_with_id(self):
        """Test NotFoundError with ID."""
        error = NotFoundError("Vector not found", id="vec-123")
        assert str(error) == "Vector not found"
        assert error.code == "NOT_FOUND"
        assert error.id == "vec-123"

    def test_repr(self):
        """Test NotFoundError repr."""
        error = NotFoundError("Not found", id="vec-456")
        assert "NotFoundError" in repr(error)
        assert "vec-456" in repr(error)


class TestProtoConversions:
    """Tests for proto enum availability."""

    def test_proto_distance_function_exists(self):
        """Test that proto DistanceFunction enum values exist."""
        assert hasattr(ProtoDistanceFunction, "DISTANCE_FUNCTION_UNSPECIFIED")
        assert hasattr(ProtoDistanceFunction, "DISTANCE_FUNCTION_EUCLIDEAN")
        assert hasattr(ProtoDistanceFunction, "DISTANCE_FUNCTION_COSINE")
        assert hasattr(ProtoDistanceFunction, "DISTANCE_FUNCTION_INNER_PRODUCT")

    def test_proto_vector_index_type_exists(self):
        """Test that proto VectorIndexType enum values exist."""
        assert hasattr(ProtoVectorIndexType, "VECTOR_INDEX_TYPE_UNSPECIFIED")
        assert hasattr(ProtoVectorIndexType, "VECTOR_INDEX_TYPE_BRUTE_FORCE")
        assert hasattr(ProtoVectorIndexType, "VECTOR_INDEX_TYPE_HNSW")
