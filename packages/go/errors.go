package liteparse

import "fmt"

// ParseError is returned when the LiteParse CLI exits with an error or emits invalid output.
type ParseError struct {
	Message  string
	Stderr   string
	ExitCode int
}

func (e *ParseError) Error() string {
	if e == nil {
		return ""
	}
	if e.ExitCode != 0 {
		return fmt.Sprintf("%s (exit code %d)", e.Message, e.ExitCode)
	}
	return e.Message
}

// CLINotFoundError is returned when the LiteParse CLI cannot be found locally.
type CLINotFoundError struct {
	Message string
}

func (e *CLINotFoundError) Error() string {
	if e == nil || e.Message == "" {
		return "liteparse CLI not found"
	}
	return e.Message
}
