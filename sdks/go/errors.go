package norikv

import (
	"fmt"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

// ErrorCode represents a NoriKV error code.
type ErrorCode string

const (
	ErrorCodeNotLeader        ErrorCode = "NOT_LEADER"
	ErrorCodeAlreadyExists    ErrorCode = "ALREADY_EXISTS"
	ErrorCodeVersionMismatch  ErrorCode = "VERSION_MISMATCH"
	ErrorCodeNotFound         ErrorCode = "NOT_FOUND"
	ErrorCodeUnavailable      ErrorCode = "UNAVAILABLE"
	ErrorCodeDeadlineExceeded ErrorCode = "DEADLINE_EXCEEDED"
	ErrorCodeInvalidArgument  ErrorCode = "INVALID_ARGUMENT"
	ErrorCodeConnectionError  ErrorCode = "CONNECTION_ERROR"
	ErrorCodeNoNodesAvailable ErrorCode = "NO_NODES_AVAILABLE"
	ErrorCodeRetryExhausted   ErrorCode = "RETRY_EXHAUSTED"
)

// Error is the base error type for all NoriKV errors.
type Error struct {
	Code    ErrorCode
	Message string
	Cause   error
}

// Error implements the error interface.
func (e *Error) Error() string {
	if e.Cause != nil {
		return fmt.Sprintf("%s: %s (caused by: %v)", e.Code, e.Message, e.Cause)
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}

// Unwrap returns the underlying error.
func (e *Error) Unwrap() error {
	return e.Cause
}

// NotLeaderError indicates the contacted node is not the leader for the requested shard.
// Client should retry on the leader node indicated in LeaderHint.
type NotLeaderError struct {
	Code       ErrorCode
	Message    string
	Cause      error
	LeaderHint string // Address of the current leader (if known)
	ShardID    uint32 // Shard ID that was accessed
}

// NewNotLeaderError creates a new NotLeaderError.
func NewNotLeaderError(message string, leaderHint string, shardID uint32) *NotLeaderError {
	return &NotLeaderError{
		Code:       ErrorCodeNotLeader,
		Message:    message,
		LeaderHint: leaderHint,
		ShardID:    shardID,
	}
}

// Error implements the error interface.
func (e *NotLeaderError) Error() string {
	if e.Cause != nil {
		return fmt.Sprintf("%s: %s (caused by: %v)", e.Code, e.Message, e.Cause)
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}

// Unwrap returns the underlying error.
func (e *NotLeaderError) Unwrap() error {
	return e.Cause
}

// AlreadyExistsError indicates the key already exists (for IfNotExists operations).
type AlreadyExistsError struct {
	Code    ErrorCode
	Message string
	Cause   error
	Key     []byte // The key that already exists
}

// NewAlreadyExistsError creates a new AlreadyExistsError.
func NewAlreadyExistsError(message string, key []byte) *AlreadyExistsError {
	return &AlreadyExistsError{
		Code:    ErrorCodeAlreadyExists,
		Message: message,
		Key:     key,
	}
}

// Error implements the error interface.
func (e *AlreadyExistsError) Error() string {
	if e.Cause != nil {
		return fmt.Sprintf("%s: %s (caused by: %v)", e.Code, e.Message, e.Cause)
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}

// Unwrap returns the underlying error.
func (e *AlreadyExistsError) Unwrap() error {
	return e.Cause
}

// NotFoundError indicates the requested resource was not found.
type NotFoundError struct {
	Code    ErrorCode
	Message string
	Cause   error
	Key     string // The key that was not found
}

// NewNotFoundError creates a new NotFoundError.
func NewNotFoundError(key string) *NotFoundError {
	return &NotFoundError{
		Code:    ErrorCodeNotFound,
		Message: fmt.Sprintf("Key not found: %s", key),
		Key:     key,
	}
}

// Error implements the error interface.
func (e *NotFoundError) Error() string {
	if e.Cause != nil {
		return fmt.Sprintf("%s: %s (caused by: %v)", e.Code, e.Message, e.Cause)
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}

// Unwrap returns the underlying error.
func (e *NotFoundError) Unwrap() error {
	return e.Cause
}

// VersionMismatchError indicates version mismatch (for IfMatchVersion operations).
type VersionMismatchError struct {
	Code            ErrorCode
	Message         string
	Cause           error
	Key             []byte   // The key being accessed
	ExpectedVersion *Version // Expected version
	ActualVersion   *Version // Actual current version
}

// NewVersionMismatchError creates a new VersionMismatchError.
func NewVersionMismatchError(message string, key []byte, expected, actual *Version) *VersionMismatchError {
	return &VersionMismatchError{
		Code:            ErrorCodeVersionMismatch,
		Message:         message,
		Key:             key,
		ExpectedVersion: expected,
		ActualVersion:   actual,
	}
}

// Error implements the error interface.
func (e *VersionMismatchError) Error() string {
	if e.Cause != nil {
		return fmt.Sprintf("%s: %s (caused by: %v)", e.Code, e.Message, e.Cause)
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}

// Unwrap returns the underlying error.
func (e *VersionMismatchError) Unwrap() error {
	return e.Cause
}

// UnavailableError indicates the service is temporarily unavailable.
// Client should retry with backoff.
type UnavailableError struct {
	Code    ErrorCode
	Message string
	Cause   error
}

// NewUnavailableError creates a new UnavailableError.
func NewUnavailableError(message string) *UnavailableError {
	return &UnavailableError{
		Code:    ErrorCodeUnavailable,
		Message: message,
	}
}

// Error implements the error interface.
func (e *UnavailableError) Error() string {
	if e.Cause != nil {
		return fmt.Sprintf("%s: %s (caused by: %v)", e.Code, e.Message, e.Cause)
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}

// Unwrap returns the underlying error.
func (e *UnavailableError) Unwrap() error {
	return e.Cause
}

// DeadlineExceededError indicates the request exceeded the deadline/timeout.
type DeadlineExceededError struct {
	Code      ErrorCode
	Message   string
	Cause     error
	TimeoutMs int // Timeout that was exceeded
}

// NewDeadlineExceededError creates a new DeadlineExceededError.
func NewDeadlineExceededError(message string, timeoutMs int) *DeadlineExceededError {
	return &DeadlineExceededError{
		Code:      ErrorCodeDeadlineExceeded,
		Message:   message,
		TimeoutMs: timeoutMs,
	}
}

// Error implements the error interface.
func (e *DeadlineExceededError) Error() string {
	if e.Cause != nil {
		return fmt.Sprintf("%s: %s (caused by: %v)", e.Code, e.Message, e.Cause)
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}

// Unwrap returns the underlying error.
func (e *DeadlineExceededError) Unwrap() error {
	return e.Cause
}

// InvalidArgumentError indicates invalid request parameters.
type InvalidArgumentError struct {
	Code    ErrorCode
	Message string
	Cause   error
}

// NewInvalidArgumentError creates a new InvalidArgumentError.
func NewInvalidArgumentError(message string) *InvalidArgumentError {
	return &InvalidArgumentError{
		Code:    ErrorCodeInvalidArgument,
		Message: message,
	}
}

// Error implements the error interface.
func (e *InvalidArgumentError) Error() string {
	if e.Cause != nil {
		return fmt.Sprintf("%s: %s (caused by: %v)", e.Code, e.Message, e.Cause)
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}

// Unwrap returns the underlying error.
func (e *InvalidArgumentError) Unwrap() error {
	return e.Cause
}

// ConnectionError indicates connection failure to the cluster.
type ConnectionError struct {
	Code    ErrorCode
	Message string
	Cause   error
	Address string // Address that failed to connect
}

// NewConnectionError creates a new ConnectionError.
func NewConnectionError(message string, address string) *ConnectionError {
	return &ConnectionError{
		Code:    ErrorCodeConnectionError,
		Message: message,
		Address: address,
	}
}

// Error implements the error interface.
func (e *ConnectionError) Error() string {
	if e.Cause != nil {
		return fmt.Sprintf("%s: %s (caused by: %v)", e.Code, e.Message, e.Cause)
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}

// Unwrap returns the underlying error.
func (e *ConnectionError) Unwrap() error {
	return e.Cause
}

// NoNodesAvailableError indicates no nodes are available in the cluster.
type NoNodesAvailableError struct {
	Code    ErrorCode
	Message string
	Cause   error
}

// NewNoNodesAvailableError creates a new NoNodesAvailableError.
func NewNoNodesAvailableError() *NoNodesAvailableError {
	return &NoNodesAvailableError{
		Code:    ErrorCodeNoNodesAvailable,
		Message: "No nodes available in the cluster",
	}
}

// Error implements the error interface.
func (e *NoNodesAvailableError) Error() string {
	if e.Cause != nil {
		return fmt.Sprintf("%s: %s (caused by: %v)", e.Code, e.Message, e.Cause)
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}

// Unwrap returns the underlying error.
func (e *NoNodesAvailableError) Unwrap() error {
	return e.Cause
}

// RetryExhaustedError indicates maximum retry attempts have been exhausted.
type RetryExhaustedError struct {
	Code      ErrorCode
	Message   string
	Cause     error
	Attempts  int   // Number of attempts made
	LastError error // Last error encountered
}

// NewRetryExhaustedError creates a new RetryExhaustedError.
func NewRetryExhaustedError(attempts int, lastError error) *RetryExhaustedError {
	message := fmt.Sprintf("Retry exhausted after %d attempts", attempts)
	if lastError != nil {
		message = fmt.Sprintf("%s: %v", message, lastError)
	}
	return &RetryExhaustedError{
		Code:      ErrorCodeRetryExhausted,
		Message:   message,
		Cause:     lastError,
		Attempts:  attempts,
		LastError: lastError,
	}
}

// Error implements the error interface.
func (e *RetryExhaustedError) Error() string {
	if e.Cause != nil {
		return fmt.Sprintf("%s: %s (caused by: %v)", e.Code, e.Message, e.Cause)
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}

// Unwrap returns the underlying error.
func (e *RetryExhaustedError) Unwrap() error {
	return e.Cause
}

// IsRetryable returns true if the error is retryable.
func IsRetryable(err error) bool {
	switch err.(type) {
	case *NotLeaderError:
		return true
	case *UnavailableError:
		return true
	case *ConnectionError:
		return true
	default:
		return false
	}
}

// IsNotLeader returns true if the error is a NotLeaderError.
func IsNotLeader(err error) bool {
	_, ok := err.(*NotLeaderError)
	return ok
}

// FromGRPCError converts a gRPC error to a NoriKV error.
func FromGRPCError(err error, md metadata.MD) error {
	if err == nil {
		return nil
	}

	// Get gRPC status
	st, ok := status.FromError(err)
	if !ok {
		return &Error{
			Code:    ErrorCodeConnectionError,
			Message: err.Error(),
			Cause:   err,
		}
	}

	code := st.Code()
	message := st.Message()

	// Extract leader hint from metadata if present
	var leaderHint string
	if md != nil {
		if hints := md.Get("leader-hint"); len(hints) > 0 {
			leaderHint = hints[0]
		}
	}

	// Map gRPC status codes to NoriKV errors
	// See: https://grpc.github.io/grpc/core/md_doc_statuscodes.html
	switch code {
	case codes.Unavailable:
		// Check if it's a NOT_LEADER error
		if contains(message, "NOT_LEADER") {
			return NewNotLeaderError(message, leaderHint, 0)
		}
		return NewUnavailableError(message)

	case codes.DeadlineExceeded:
		return NewDeadlineExceededError(message, 0)

	case codes.InvalidArgument:
		return NewInvalidArgumentError(message)

	case codes.AlreadyExists:
		return NewAlreadyExistsError(message, nil)

	case codes.FailedPrecondition:
		// Version mismatch
		return NewVersionMismatchError(message, nil, nil, nil)

	case codes.Canceled:
		return &Error{
			Code:    ErrorCodeDeadlineExceeded,
			Message: message,
			Cause:   err,
		}

	default:
		return &Error{
			Code:    ErrorCode(code.String()),
			Message: message,
			Cause:   err,
		}
	}
}

// Helper function to check if a string contains a substring.
func contains(s, substr string) bool {
	return len(s) >= len(substr) && (s == substr || len(s) > len(substr) && findSubstring(s, substr))
}

func findSubstring(s, substr string) bool {
	for i := 0; i <= len(s)-len(substr); i++ {
		if s[i:i+len(substr)] == substr {
			return true
		}
	}
	return false
}
