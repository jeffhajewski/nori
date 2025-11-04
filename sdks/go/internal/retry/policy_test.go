package retry

import (
	"context"
	"errors"
	"testing"
	"time"
)

func TestNewPolicy(t *testing.T) {
	t.Run("creates policy with defaults", func(t *testing.T) {
		p := NewPolicy()
		if p.MaxAttempts() != 3 {
			t.Errorf("Expected maxAttempts=3, got %d", p.MaxAttempts())
		}
		if p.InitialDelay() != 10 {
			t.Errorf("Expected initialDelay=10, got %d", p.InitialDelay())
		}
		if p.MaxDelay() != 1000 {
			t.Errorf("Expected maxDelay=1000, got %d", p.MaxDelay())
		}
		if p.BackoffMultiplier() != 2.0 {
			t.Errorf("Expected backoffMultiplier=2.0, got %f", p.BackoffMultiplier())
		}
		if p.Jitter() != 100 {
			t.Errorf("Expected jitter=100, got %d", p.Jitter())
		}
		if !p.RetryOnNotLeader() {
			t.Error("Expected retryOnNotLeader=true")
		}
	})

	t.Run("creates policy with custom options", func(t *testing.T) {
		p := NewPolicy(
			WithMaxAttempts(5),
			WithInitialDelay(50),
			WithMaxDelay(5000),
			WithBackoffMultiplier(3.0),
			WithJitter(200),
			WithRetryOnNotLeader(false),
		)
		if p.MaxAttempts() != 5 {
			t.Errorf("Expected maxAttempts=5, got %d", p.MaxAttempts())
		}
		if p.InitialDelay() != 50 {
			t.Errorf("Expected initialDelay=50, got %d", p.InitialDelay())
		}
		if p.MaxDelay() != 5000 {
			t.Errorf("Expected maxDelay=5000, got %d", p.MaxDelay())
		}
		if p.BackoffMultiplier() != 3.0 {
			t.Errorf("Expected backoffMultiplier=3.0, got %f", p.BackoffMultiplier())
		}
		if p.Jitter() != 200 {
			t.Errorf("Expected jitter=200, got %d", p.Jitter())
		}
		if p.RetryOnNotLeader() {
			t.Error("Expected retryOnNotLeader=false")
		}
	})
}

func TestExecute(t *testing.T) {
	t.Run("succeeds on first attempt", func(t *testing.T) {
		p := NewPolicy()
		ctx := context.Background()

		attempts := 0
		err := p.Execute(ctx, func() error {
			attempts++
			return nil
		})

		if err != nil {
			t.Errorf("Expected no error, got %v", err)
		}
		if attempts != 1 {
			t.Errorf("Expected 1 attempt, got %d", attempts)
		}
	})

	t.Run("succeeds on second attempt", func(t *testing.T) {
		p := NewPolicy(WithInitialDelay(1), WithJitter(0))
		ctx := context.Background()

		attempts := 0
		err := p.Execute(ctx, func() error {
			attempts++
			if attempts == 1 {
				return errors.New("UNAVAILABLE: service temporarily unavailable")
			}
			return nil
		})

		if err != nil {
			t.Errorf("Expected no error, got %v", err)
		}
		if attempts != 2 {
			t.Errorf("Expected 2 attempts, got %d", attempts)
		}
	})

	t.Run("exhausts retries", func(t *testing.T) {
		p := NewPolicy(WithMaxAttempts(3), WithInitialDelay(1), WithJitter(0))
		ctx := context.Background()

		attempts := 0
		err := p.Execute(ctx, func() error {
			attempts++
			return errors.New("UNAVAILABLE: always failing")
		})

		if err == nil {
			t.Error("Expected error, got nil")
		}
		if attempts != 3 {
			t.Errorf("Expected 3 attempts, got %d", attempts)
		}
	})

	t.Run("respects context cancellation", func(t *testing.T) {
		p := NewPolicy(WithMaxAttempts(10), WithInitialDelay(100), WithJitter(0))
		ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
		defer cancel()

		attempts := 0
		err := p.Execute(ctx, func() error {
			attempts++
			return errors.New("UNAVAILABLE: always failing")
		})

		if err == nil {
			t.Error("Expected error, got nil")
		}
		// Should stop early due to context cancellation
		if attempts >= 10 {
			t.Errorf("Expected < 10 attempts due to context cancellation, got %d", attempts)
		}
	})

	t.Run("does not retry non-retryable errors", func(t *testing.T) {
		p := NewPolicy()
		ctx := context.Background()

		attempts := 0
		err := p.Execute(ctx, func() error {
			attempts++
			return errors.New("INVALID_ARGUMENT: bad request")
		})

		if err == nil {
			t.Error("Expected error, got nil")
		}
		if attempts != 1 {
			t.Errorf("Expected 1 attempt (no retry for non-retryable error), got %d", attempts)
		}
	})

	t.Run("retries NOT_LEADER when enabled", func(t *testing.T) {
		p := NewPolicy(WithRetryOnNotLeader(true), WithInitialDelay(1), WithJitter(0))
		ctx := context.Background()

		attempts := 0
		err := p.Execute(ctx, func() error {
			attempts++
			if attempts == 1 {
				return errors.New("NOT_LEADER: not the leader")
			}
			return nil
		})

		if err != nil {
			t.Errorf("Expected no error, got %v", err)
		}
		if attempts != 2 {
			t.Errorf("Expected 2 attempts, got %d", attempts)
		}
	})

	t.Run("does not retry NOT_LEADER when disabled", func(t *testing.T) {
		p := NewPolicy(WithRetryOnNotLeader(false))
		ctx := context.Background()

		attempts := 0
		err := p.Execute(ctx, func() error {
			attempts++
			return errors.New("NOT_LEADER: not the leader")
		})

		if err == nil {
			t.Error("Expected error, got nil")
		}
		if attempts != 1 {
			t.Errorf("Expected 1 attempt (no retry for NOT_LEADER), got %d", attempts)
		}
	})
}

func TestExecuteWithResult(t *testing.T) {
	t.Run("returns result on success", func(t *testing.T) {
		p := NewPolicy()
		ctx := context.Background()

		result, err := ExecuteWithResult(ctx, p, func() (string, error) {
			return "success", nil
		})

		if err != nil {
			t.Errorf("Expected no error, got %v", err)
		}
		if result != "success" {
			t.Errorf("Expected result='success', got '%s'", result)
		}
	})

	t.Run("returns result after retry", func(t *testing.T) {
		p := NewPolicy(WithInitialDelay(1), WithJitter(0))
		ctx := context.Background()

		attempts := 0
		result, err := ExecuteWithResult(ctx, p, func() (int, error) {
			attempts++
			if attempts == 1 {
				return 0, errors.New("UNAVAILABLE: retry me")
			}
			return 42, nil
		})

		if err != nil {
			t.Errorf("Expected no error, got %v", err)
		}
		if result != 42 {
			t.Errorf("Expected result=42, got %d", result)
		}
		if attempts != 2 {
			t.Errorf("Expected 2 attempts, got %d", attempts)
		}
	})

	t.Run("returns last result and error on failure", func(t *testing.T) {
		p := NewPolicy(WithMaxAttempts(2), WithInitialDelay(1), WithJitter(0))
		ctx := context.Background()

		attempts := 0
		result, err := ExecuteWithResult(ctx, p, func() (int, error) {
			attempts++
			return attempts * 10, errors.New("UNAVAILABLE: always failing")
		})

		if err == nil {
			t.Error("Expected error, got nil")
		}
		// Should return result from last attempt
		if result != 20 {
			t.Errorf("Expected result=20 (from last attempt), got %d", result)
		}
		if attempts != 2 {
			t.Errorf("Expected 2 attempts, got %d", attempts)
		}
	})
}

func TestCalculateDelay(t *testing.T) {
	t.Run("calculates exponential backoff", func(t *testing.T) {
		p := NewPolicy(
			WithInitialDelay(100),
			WithBackoffMultiplier(2.0),
			WithJitter(0), // No jitter for predictable testing
		)

		// Attempt 0: 100ms
		delay0 := p.calculateDelay(0)
		if delay0 < 100*time.Millisecond || delay0 > 100*time.Millisecond {
			t.Errorf("Expected delay ~100ms for attempt 0, got %v", delay0)
		}

		// Attempt 1: 200ms (100 * 2)
		delay1 := p.calculateDelay(1)
		if delay1 < 200*time.Millisecond || delay1 > 200*time.Millisecond {
			t.Errorf("Expected delay ~200ms for attempt 1, got %v", delay1)
		}

		// Attempt 2: 400ms (100 * 2 * 2)
		delay2 := p.calculateDelay(2)
		if delay2 < 400*time.Millisecond || delay2 > 400*time.Millisecond {
			t.Errorf("Expected delay ~400ms for attempt 2, got %v", delay2)
		}
	})

	t.Run("caps at max delay", func(t *testing.T) {
		p := NewPolicy(
			WithInitialDelay(100),
			WithMaxDelay(500),
			WithBackoffMultiplier(2.0),
			WithJitter(0),
		)

		// Attempt 5 would be 100 * 2^5 = 3200ms, but should be capped at 500ms
		delay5 := p.calculateDelay(5)
		if delay5 != 500*time.Millisecond {
			t.Errorf("Expected delay capped at 500ms, got %v", delay5)
		}
	})

	t.Run("adds jitter", func(t *testing.T) {
		p := NewPolicy(
			WithInitialDelay(100),
			WithJitter(50),
		)

		// With jitter, delay should be between 100-150ms
		delay := p.calculateDelay(0)
		if delay < 100*time.Millisecond || delay > 150*time.Millisecond {
			t.Errorf("Expected delay between 100-150ms with jitter, got %v", delay)
		}
	})
}

func TestShouldRetry(t *testing.T) {
	tests := []struct {
		name              string
		err               error
		retryOnNotLeader  bool
		expectedRetry     bool
	}{
		{"nil error", nil, true, false},
		{"UNAVAILABLE error", errors.New("UNAVAILABLE: service down"), true, true},
		{"CONNECTION_ERROR", errors.New("CONNECTION_ERROR: network failure"), true, true},
		{"NOT_LEADER with retry enabled", errors.New("NOT_LEADER: redirect"), true, true},
		{"NOT_LEADER with retry disabled", errors.New("NOT_LEADER: redirect"), false, false},
		{"INVALID_ARGUMENT", errors.New("INVALID_ARGUMENT: bad request"), true, false},
		{"ALREADY_EXISTS", errors.New("ALREADY_EXISTS: duplicate"), true, false},
		{"VERSION_MISMATCH", errors.New("VERSION_MISMATCH: conflict"), true, false},
		{"unknown error", errors.New("some random error"), true, false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			p := NewPolicy(WithRetryOnNotLeader(tt.retryOnNotLeader))
			shouldRetry := p.shouldRetry(tt.err)
			if shouldRetry != tt.expectedRetry {
				t.Errorf("Expected shouldRetry=%v, got %v for error: %v",
					tt.expectedRetry, shouldRetry, tt.err)
			}
		})
	}
}

func BenchmarkExecute(b *testing.B) {
	p := NewPolicy(WithJitter(0))
	ctx := context.Background()

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_ = p.Execute(ctx, func() error {
			return nil
		})
	}
}

func BenchmarkCalculateDelay(b *testing.B) {
	p := NewPolicy(WithJitter(0))

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_ = p.calculateDelay(i % 10)
	}
}
