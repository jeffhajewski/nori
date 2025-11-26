package norikv

import (
	"testing"
)

func TestDistanceFunctionString(t *testing.T) {
	tests := []struct {
		distance DistanceFunction
		want     string
	}{
		{DistanceEuclidean, "euclidean"},
		{DistanceCosine, "cosine"},
		{DistanceInnerProduct, "inner_product"},
		{DistanceFunction(99), "unknown"},
	}

	for _, tt := range tests {
		t.Run(tt.want, func(t *testing.T) {
			got := tt.distance.String()
			if got != tt.want {
				t.Errorf("DistanceFunction.String() = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestVectorIndexTypeString(t *testing.T) {
	tests := []struct {
		indexType VectorIndexType
		want      string
	}{
		{VectorIndexBruteForce, "brute_force"},
		{VectorIndexHNSW, "hnsw"},
		{VectorIndexType(99), "unknown"},
	}

	for _, tt := range tests {
		t.Run(tt.want, func(t *testing.T) {
			got := tt.indexType.String()
			if got != tt.want {
				t.Errorf("VectorIndexType.String() = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestVectorCreateIndexValidation(t *testing.T) {
	client := &Client{
		config: DefaultClientConfig([]string{"localhost:9001"}),
	}

	tests := []struct {
		name       string
		namespace  string
		dimensions uint32
		wantErr    string
	}{
		{
			name:       "empty namespace",
			namespace:  "",
			dimensions: 128,
			wantErr:    "namespace cannot be empty",
		},
		{
			name:       "zero dimensions",
			namespace:  "test",
			dimensions: 0,
			wantErr:    "dimensions must be greater than 0",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			_, err := client.VectorCreateIndex(
				nil, // nil context for validation-only test
				tt.namespace,
				tt.dimensions,
				DistanceEuclidean,
				VectorIndexHNSW,
				nil,
			)
			if err == nil {
				t.Error("expected error, got nil")
				return
			}
			if !containsSubstring(err.Error(), tt.wantErr) {
				t.Errorf("error = %q, want to contain %q", err.Error(), tt.wantErr)
			}
		})
	}
}

func TestVectorInsertValidation(t *testing.T) {
	client := &Client{
		config: DefaultClientConfig([]string{"localhost:9001"}),
	}

	tests := []struct {
		name      string
		namespace string
		id        string
		vector    []float32
		wantErr   string
	}{
		{
			name:      "empty namespace",
			namespace: "",
			id:        "vec1",
			vector:    []float32{1.0, 2.0},
			wantErr:   "namespace cannot be empty",
		},
		{
			name:      "empty id",
			namespace: "test",
			id:        "",
			vector:    []float32{1.0, 2.0},
			wantErr:   "id cannot be empty",
		},
		{
			name:      "empty vector",
			namespace: "test",
			id:        "vec1",
			vector:    []float32{},
			wantErr:   "vector cannot be empty",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			_, err := client.VectorInsert(
				nil,
				tt.namespace,
				tt.id,
				tt.vector,
				nil,
			)
			if err == nil {
				t.Error("expected error, got nil")
				return
			}
			if !containsSubstring(err.Error(), tt.wantErr) {
				t.Errorf("error = %q, want to contain %q", err.Error(), tt.wantErr)
			}
		})
	}
}

func TestVectorSearchValidation(t *testing.T) {
	client := &Client{
		config: DefaultClientConfig([]string{"localhost:9001"}),
	}

	tests := []struct {
		name      string
		namespace string
		query     []float32
		k         uint32
		wantErr   string
	}{
		{
			name:      "empty namespace",
			namespace: "",
			query:     []float32{1.0, 2.0},
			k:         5,
			wantErr:   "namespace cannot be empty",
		},
		{
			name:      "empty query",
			namespace: "test",
			query:     []float32{},
			k:         5,
			wantErr:   "query vector cannot be empty",
		},
		{
			name:      "zero k",
			namespace: "test",
			query:     []float32{1.0, 2.0},
			k:         0,
			wantErr:   "k must be greater than 0",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			_, err := client.VectorSearch(
				nil,
				tt.namespace,
				tt.query,
				tt.k,
				nil,
			)
			if err == nil {
				t.Error("expected error, got nil")
				return
			}
			if !containsSubstring(err.Error(), tt.wantErr) {
				t.Errorf("error = %q, want to contain %q", err.Error(), tt.wantErr)
			}
		})
	}
}

func TestVectorGetValidation(t *testing.T) {
	client := &Client{
		config: DefaultClientConfig([]string{"localhost:9001"}),
	}

	tests := []struct {
		name      string
		namespace string
		id        string
		wantErr   string
	}{
		{
			name:      "empty namespace",
			namespace: "",
			id:        "vec1",
			wantErr:   "namespace cannot be empty",
		},
		{
			name:      "empty id",
			namespace: "test",
			id:        "",
			wantErr:   "id cannot be empty",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			_, err := client.VectorGet(nil, tt.namespace, tt.id)
			if err == nil {
				t.Error("expected error, got nil")
				return
			}
			if !containsSubstring(err.Error(), tt.wantErr) {
				t.Errorf("error = %q, want to contain %q", err.Error(), tt.wantErr)
			}
		})
	}
}

func TestVectorDeleteValidation(t *testing.T) {
	client := &Client{
		config: DefaultClientConfig([]string{"localhost:9001"}),
	}

	tests := []struct {
		name      string
		namespace string
		id        string
		wantErr   string
	}{
		{
			name:      "empty namespace",
			namespace: "",
			id:        "vec1",
			wantErr:   "namespace cannot be empty",
		},
		{
			name:      "empty id",
			namespace: "test",
			id:        "",
			wantErr:   "id cannot be empty",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			_, err := client.VectorDelete(nil, tt.namespace, tt.id, nil)
			if err == nil {
				t.Error("expected error, got nil")
				return
			}
			if !containsSubstring(err.Error(), tt.wantErr) {
				t.Errorf("error = %q, want to contain %q", err.Error(), tt.wantErr)
			}
		})
	}
}

func TestVectorDropIndexValidation(t *testing.T) {
	client := &Client{
		config: DefaultClientConfig([]string{"localhost:9001"}),
	}

	_, err := client.VectorDropIndex(nil, "", nil)
	if err == nil {
		t.Error("expected error for empty namespace, got nil")
		return
	}
	if !containsSubstring(err.Error(), "namespace cannot be empty") {
		t.Errorf("error = %q, want to contain 'namespace cannot be empty'", err.Error())
	}
}

// Helper to check substring
func containsSubstring(s, substr string) bool {
	return len(s) >= len(substr) && findSubstr(s, substr)
}

func findSubstr(s, substr string) bool {
	for i := 0; i <= len(s)-len(substr); i++ {
		if s[i:i+len(substr)] == substr {
			return true
		}
	}
	return false
}
