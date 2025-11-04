// Package retry implements exponential backoff retry logic for NoriKV client.
package retry

import (
	"context"
	"math/rand"
	"time"
)

// Policy defines a retry policy with exponential backoff.
type Policy struct {
	maxAttempts       int
	initialDelayMs    int
	maxDelayMs        int
	backoffMultiplier float64
	jitterMs          int
	retryOnNotLeader  bool
}

// Option is a functional option for configuring a retry policy.
type Option func(*Policy)

// WithMaxAttempts sets the maximum number of retry attempts.
func WithMaxAttempts(attempts int) Option {
	return func(p *Policy) {
		p.maxAttempts = attempts
	}
}

// WithInitialDelay sets the initial delay in milliseconds.
func WithInitialDelay(delayMs int) Option {
	return func(p *Policy) {
		p.initialDelayMs = delayMs
	}
}

// WithMaxDelay sets the maximum delay in milliseconds.
func WithMaxDelay(delayMs int) Option {
	return func(p *Policy) {
		p.maxDelayMs = delayMs
	}
}

// WithBackoffMultiplier sets the backoff multiplier.
func WithBackoffMultiplier(multiplier float64) Option {
	return func(p *Policy) {
		p.backoffMultiplier = multiplier
	}
}

// WithJitter sets the jitter in milliseconds.
func WithJitter(jitterMs int) Option {
	return func(p *Policy) {
		p.jitterMs = jitterMs
	}
}

// WithRetryOnNotLeader enables/disables retry on NOT_LEADER errors.
func WithRetryOnNotLeader(retry bool) Option {
	return func(p *Policy) {
		p.retryOnNotLeader = retry
	}
}

// NewPolicy creates a new retry policy with the given options.
// If no options are provided, defaults to:
// - maxAttempts: 3
// - initialDelayMs: 10
// - maxDelayMs: 1000
// - backoffMultiplier: 2.0
// - jitterMs: 100
// - retryOnNotLeader: true
func NewPolicy(opts ...Option) *Policy {
	p := &Policy{
		maxAttempts:       3,
		initialDelayMs:    10,
		maxDelayMs:        1000,
		backoffMultiplier: 2.0,
		jitterMs:          100,
		retryOnNotLeader:  true,
	}

	for _, opt := range opts {
		opt(p)
	}

	return p
}

// Execute executes a function with retry logic.
// The function is retried if it returns an error that is retryable.
// Returns the result of the function or the last error encountered.
func (p *Policy) Execute(ctx context.Context, fn func() error) error {
	var lastErr error

	for attempt := 0; attempt < p.maxAttempts; attempt++ {
		// Check context before each attempt
		select {
		case <-ctx.Done():
			if lastErr != nil {
				return lastErr
			}
			return ctx.Err()
		default:
		}

		// Execute the function
		err := fn()
		if err == nil {
			return nil // Success!
		}

		lastErr = err

		// Don't retry on last attempt
		if attempt == p.maxAttempts-1 {
			break
		}

		// Check if error is retryable
		if !p.shouldRetry(err) {
			return err
		}

		// Calculate delay with exponential backoff
		delay := p.calculateDelay(attempt)

		// Wait before next attempt (respecting context cancellation)
		select {
		case <-ctx.Done():
			return lastErr
		case <-time.After(delay):
			// Continue to next attempt
		}
	}

	return lastErr
}

// ExecuteWithResult executes a function with retry logic that returns a result.
// The function is retried if it returns an error that is retryable.
// Returns the result of the function or the last error encountered.
func ExecuteWithResult[T any](ctx context.Context, p *Policy, fn func() (T, error)) (T, error) {
	var result T
	var lastErr error

	for attempt := 0; attempt < p.maxAttempts; attempt++ {
		// Check context before each attempt
		select {
		case <-ctx.Done():
			if lastErr != nil {
				return result, lastErr
			}
			return result, ctx.Err()
		default:
		}

		// Execute the function
		res, err := fn()
		if err == nil {
			return res, nil // Success!
		}

		result = res
		lastErr = err

		// Don't retry on last attempt
		if attempt == p.maxAttempts-1 {
			break
		}

		// Check if error is retryable
		if !p.shouldRetry(err) {
			return result, err
		}

		// Calculate delay with exponential backoff
		delay := p.calculateDelay(attempt)

		// Wait before next attempt (respecting context cancellation)
		select {
		case <-ctx.Done():
			return result, lastErr
		case <-time.After(delay):
			// Continue to next attempt
		}
	}

	return result, lastErr
}

// shouldRetry determines if an error should trigger a retry.
func (p *Policy) shouldRetry(err error) bool {
	if err == nil {
		return false
	}

	// Import would create circular dependency, so we use type assertion
	// to check for retryable errors based on their type name
	type retryableError interface {
		Error() string
	}

	// Check error type by examining the error string
	// This is a simple heuristic - in production, you'd use proper error types
	errStr := err.Error()

	// Retryable errors:
	// - UNAVAILABLE
	// - CONNECTION_ERROR
	// - NOT_LEADER (if enabled)
	if contains(errStr, "UNAVAILABLE") {
		return true
	}
	if contains(errStr, "CONNECTION_ERROR") {
		return true
	}
	if p.retryOnNotLeader && contains(errStr, "NOT_LEADER") {
		return true
	}

	return false
}

// calculateDelay calculates the delay for the given attempt number.
// Uses exponential backoff with jitter to avoid thundering herd.
func (p *Policy) calculateDelay(attempt int) time.Duration {
	// Calculate base delay: initialDelay * (multiplier ^ attempt)
	baseDelay := float64(p.initialDelayMs)
	for i := 0; i < attempt; i++ {
		baseDelay *= p.backoffMultiplier
	}

	// Cap at max delay
	if baseDelay > float64(p.maxDelayMs) {
		baseDelay = float64(p.maxDelayMs)
	}

	// Add jitter to avoid thundering herd
	jitter := 0
	if p.jitterMs > 0 {
		jitter = rand.Intn(p.jitterMs)
	}

	totalDelayMs := int(baseDelay) + jitter
	return time.Duration(totalDelayMs) * time.Millisecond
}

// contains checks if a string contains a substring (helper function).
func contains(s, substr string) bool {
	return len(s) >= len(substr) && findSubstring(s, substr)
}

func findSubstring(s, substr string) bool {
	if len(s) < len(substr) {
		return false
	}
	for i := 0; i <= len(s)-len(substr); i++ {
		if s[i:i+len(substr)] == substr {
			return true
		}
	}
	return false
}

// MaxAttempts returns the maximum number of retry attempts.
func (p *Policy) MaxAttempts() int {
	return p.maxAttempts
}

// InitialDelay returns the initial delay in milliseconds.
func (p *Policy) InitialDelay() int {
	return p.initialDelayMs
}

// MaxDelay returns the maximum delay in milliseconds.
func (p *Policy) MaxDelay() int {
	return p.maxDelayMs
}

// BackoffMultiplier returns the backoff multiplier.
func (p *Policy) BackoffMultiplier() float64 {
	return p.backoffMultiplier
}

// Jitter returns the jitter in milliseconds.
func (p *Policy) Jitter() int {
	return p.jitterMs
}

// RetryOnNotLeader returns whether retry on NOT_LEADER is enabled.
func (p *Policy) RetryOnNotLeader() bool {
	return p.retryOnNotLeader
}
