/**
 * Unit tests for vector operations.
 *
 * These tests validate type conversions and validation logic
 * without requiring a server connection.
 */

import { describe, it, expect } from 'vitest';
import {
  toProtoDistanceFunction,
  toProtoVectorIndexType,
  ProtoDistanceFunction,
  ProtoVectorIndexType,
} from '@norikv/client/proto-types';
import type {
  DistanceFunction,
  VectorIndexType,
} from '@norikv/client/types';

describe('Vector Types', () => {
  describe('DistanceFunction conversion', () => {
    it('should convert euclidean', () => {
      const result = toProtoDistanceFunction('euclidean');
      expect(result).toBe(ProtoDistanceFunction.DISTANCE_FUNCTION_EUCLIDEAN);
    });

    it('should convert cosine', () => {
      const result = toProtoDistanceFunction('cosine');
      expect(result).toBe(ProtoDistanceFunction.DISTANCE_FUNCTION_COSINE);
    });

    it('should convert inner_product', () => {
      const result = toProtoDistanceFunction('inner_product');
      expect(result).toBe(ProtoDistanceFunction.DISTANCE_FUNCTION_INNER_PRODUCT);
    });

    it('should handle unknown as unspecified', () => {
      const result = toProtoDistanceFunction('unknown' as DistanceFunction);
      expect(result).toBe(ProtoDistanceFunction.DISTANCE_FUNCTION_UNSPECIFIED);
    });
  });

  describe('VectorIndexType conversion', () => {
    it('should convert brute_force', () => {
      const result = toProtoVectorIndexType('brute_force');
      expect(result).toBe(ProtoVectorIndexType.VECTOR_INDEX_TYPE_BRUTE_FORCE);
    });

    it('should convert hnsw', () => {
      const result = toProtoVectorIndexType('hnsw');
      expect(result).toBe(ProtoVectorIndexType.VECTOR_INDEX_TYPE_HNSW);
    });

    it('should handle unknown as unspecified', () => {
      const result = toProtoVectorIndexType('unknown' as VectorIndexType);
      expect(result).toBe(ProtoVectorIndexType.VECTOR_INDEX_TYPE_UNSPECIFIED);
    });
  });

  describe('DistanceFunction string values', () => {
    it('euclidean should be a valid type', () => {
      const df: DistanceFunction = 'euclidean';
      expect(df).toBe('euclidean');
    });

    it('cosine should be a valid type', () => {
      const df: DistanceFunction = 'cosine';
      expect(df).toBe('cosine');
    });

    it('inner_product should be a valid type', () => {
      const df: DistanceFunction = 'inner_product';
      expect(df).toBe('inner_product');
    });
  });

  describe('VectorIndexType string values', () => {
    it('brute_force should be a valid type', () => {
      const vit: VectorIndexType = 'brute_force';
      expect(vit).toBe('brute_force');
    });

    it('hnsw should be a valid type', () => {
      const vit: VectorIndexType = 'hnsw';
      expect(vit).toBe('hnsw');
    });
  });
});
