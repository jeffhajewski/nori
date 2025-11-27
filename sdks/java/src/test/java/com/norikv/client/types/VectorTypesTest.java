package com.norikv.client.types;

import org.junit.jupiter.api.Test;

import java.util.Arrays;
import java.util.Collections;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Unit tests for vector types.
 */
class VectorTypesTest {

    // =========================================================================
    // DistanceFunction Tests
    // =========================================================================

    @Test
    void testDistanceFunctionValues() {
        assertEquals("euclidean", DistanceFunction.EUCLIDEAN.getValue());
        assertEquals("cosine", DistanceFunction.COSINE.getValue());
        assertEquals("inner_product", DistanceFunction.INNER_PRODUCT.getValue());
    }

    @Test
    void testDistanceFunctionFromValue() {
        assertEquals(DistanceFunction.EUCLIDEAN, DistanceFunction.fromValue("euclidean"));
        assertEquals(DistanceFunction.COSINE, DistanceFunction.fromValue("cosine"));
        assertEquals(DistanceFunction.INNER_PRODUCT, DistanceFunction.fromValue("inner_product"));
    }

    @Test
    void testDistanceFunctionFromValueInvalid() {
        assertThrows(IllegalArgumentException.class, () ->
                DistanceFunction.fromValue("invalid"));
    }

    // =========================================================================
    // VectorIndexType Tests
    // =========================================================================

    @Test
    void testVectorIndexTypeValues() {
        assertEquals("brute_force", VectorIndexType.BRUTE_FORCE.getValue());
        assertEquals("hnsw", VectorIndexType.HNSW.getValue());
    }

    @Test
    void testVectorIndexTypeFromValue() {
        assertEquals(VectorIndexType.BRUTE_FORCE, VectorIndexType.fromValue("brute_force"));
        assertEquals(VectorIndexType.HNSW, VectorIndexType.fromValue("hnsw"));
    }

    @Test
    void testVectorIndexTypeFromValueInvalid() {
        assertThrows(IllegalArgumentException.class, () ->
                VectorIndexType.fromValue("invalid"));
    }

    // =========================================================================
    // VectorMatch Tests
    // =========================================================================

    @Test
    void testVectorMatchBasic() {
        VectorMatch match = new VectorMatch("vec-1", 0.5f, null);

        assertEquals("vec-1", match.getId());
        assertEquals(0.5f, match.getDistance(), 0.001);
        assertNull(match.getVector());
    }

    @Test
    void testVectorMatchWithVector() {
        List<Float> vector = Arrays.asList(0.1f, 0.2f, 0.3f);
        VectorMatch match = new VectorMatch("vec-2", 0.25f, vector);

        assertEquals("vec-2", match.getId());
        assertEquals(0.25f, match.getDistance(), 0.001);
        assertEquals(vector, match.getVector());
    }

    @Test
    void testVectorMatchNullIdThrows() {
        assertThrows(NullPointerException.class, () ->
                new VectorMatch(null, 0.5f, null));
    }

    @Test
    void testVectorMatchEquals() {
        VectorMatch match1 = new VectorMatch("vec-1", 0.5f, null);
        VectorMatch match2 = new VectorMatch("vec-1", 0.5f, null);
        VectorMatch match3 = new VectorMatch("vec-2", 0.5f, null);

        assertEquals(match1, match2);
        assertNotEquals(match1, match3);
    }

    @Test
    void testVectorMatchHashCode() {
        VectorMatch match1 = new VectorMatch("vec-1", 0.5f, null);
        VectorMatch match2 = new VectorMatch("vec-1", 0.5f, null);

        assertEquals(match1.hashCode(), match2.hashCode());
    }

    // =========================================================================
    // VectorSearchResult Tests
    // =========================================================================

    @Test
    void testVectorSearchResultEmpty() {
        VectorSearchResult result = new VectorSearchResult(Collections.emptyList(), 100);

        assertTrue(result.isEmpty());
        assertEquals(0, result.size());
        assertEquals(100, result.getSearchTimeUs());
    }

    @Test
    void testVectorSearchResultWithMatches() {
        List<VectorMatch> matches = Arrays.asList(
                new VectorMatch("vec-1", 0.1f, null),
                new VectorMatch("vec-2", 0.2f, null),
                new VectorMatch("vec-3", 0.3f, null)
        );
        VectorSearchResult result = new VectorSearchResult(matches, 500);

        assertFalse(result.isEmpty());
        assertEquals(3, result.size());
        assertEquals(500, result.getSearchTimeUs());
        assertEquals("vec-1", result.getMatches().get(0).getId());
    }

    @Test
    void testVectorSearchResultNullMatches() {
        VectorSearchResult result = new VectorSearchResult(null, 100);

        assertTrue(result.isEmpty());
        assertEquals(0, result.size());
    }

    @Test
    void testVectorSearchResultImmutable() {
        List<VectorMatch> matches = Arrays.asList(
                new VectorMatch("vec-1", 0.1f, null)
        );
        VectorSearchResult result = new VectorSearchResult(matches, 100);

        assertThrows(UnsupportedOperationException.class, () ->
                result.getMatches().add(new VectorMatch("vec-2", 0.2f, null)));
    }

    // =========================================================================
    // Vector Options Tests
    // =========================================================================

    @Test
    void testCreateVectorIndexOptionsDefault() {
        CreateVectorIndexOptions opts = CreateVectorIndexOptions.builder().build();

        assertNull(opts.getIdempotencyKey());
    }

    @Test
    void testCreateVectorIndexOptionsWithKey() {
        CreateVectorIndexOptions opts = CreateVectorIndexOptions.builder()
                .idempotencyKey("create-123")
                .build();

        assertEquals("create-123", opts.getIdempotencyKey());
    }

    @Test
    void testDropVectorIndexOptionsDefault() {
        DropVectorIndexOptions opts = DropVectorIndexOptions.builder().build();

        assertNull(opts.getIdempotencyKey());
    }

    @Test
    void testVectorInsertOptionsDefault() {
        VectorInsertOptions opts = VectorInsertOptions.builder().build();

        assertNull(opts.getIdempotencyKey());
    }

    @Test
    void testVectorDeleteOptionsDefault() {
        VectorDeleteOptions opts = VectorDeleteOptions.builder().build();

        assertNull(opts.getIdempotencyKey());
    }

    @Test
    void testVectorSearchOptionsDefault() {
        VectorSearchOptions opts = VectorSearchOptions.builder().build();

        assertFalse(opts.isIncludeVectors());
    }

    @Test
    void testVectorSearchOptionsIncludeVectors() {
        VectorSearchOptions opts = VectorSearchOptions.builder()
                .includeVectors(true)
                .build();

        assertTrue(opts.isIncludeVectors());
    }

    // =========================================================================
    // VectorNotFoundException Tests
    // =========================================================================

    @Test
    void testVectorNotFoundExceptionBasic() {
        VectorNotFoundException ex = new VectorNotFoundException("Vector not found", null);

        assertEquals("Vector not found", ex.getMessage());
        assertEquals("NOT_FOUND", ex.getCode());
        assertNull(ex.getVectorId());
    }

    @Test
    void testVectorNotFoundExceptionWithId() {
        VectorNotFoundException ex = new VectorNotFoundException("Vector not found", "vec-123");

        assertEquals("vec-123", ex.getVectorId());
    }
}
