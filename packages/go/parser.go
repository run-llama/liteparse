/*
 * Copyright 2025 coze-dev Authors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package liteparse

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"
)

// LiteParse wraps the local LiteParse CLI and exposes a Go-friendly API.
type LiteParse struct {
	cliPath string
}

// New creates a new wrapper. If cliPath is empty the CLI is auto-discovered lazily.
func New(cliPath string) *LiteParse {
	return &LiteParse{cliPath: cliPath}
}

// CLIPath returns the configured or auto-discovered CLI command string.
func (p *LiteParse) CLIPath() (string, error) {
	cmd, err := p.resolveCLICommand()
	if err != nil {
		return "", err
	}
	return strings.Join(cmd, " "), nil
}

// Parse parses a single document into structured output.
func (p *LiteParse) Parse(filePath string, opts *ParseOptions) (*ParseResult, error) {
	if _, err := os.Stat(filePath); err != nil {
		return nil, err
	}

	options := DefaultParseOptions()
	if opts != nil {
		options = mergeParseOptions(options, *opts)
	}

	cmd, err := p.resolveCLICommand()
	if err != nil {
		return nil, err
	}

	args := append(cmd, "parse", absPath(filePath))
	args = append(args, buildParseCLIArgs(options)...)

	stdout, stderr, exitCode, err := runCommand(args, options.Timeout)
	if err != nil {
		return nil, err
	}
	if exitCode != 0 {
		return nil, &ParseError{
			Message:  "parsing failed",
			Stderr:   stderr,
			ExitCode: exitCode,
		}
	}

	result, err := parseJSONResult(stdout, options.OutputFormat)
	if err != nil {
		return nil, &ParseError{
			Message: fmt.Sprintf("failed to parse CLI output: %v", err),
			Stderr:  stderr,
		}
	}
	return result, nil
}

// BatchParse parses a directory of files into an output directory.
func (p *LiteParse) BatchParse(inputDir, outputDir string, opts *BatchParseOptions) (*BatchResult, error) {
	info, err := os.Stat(inputDir)
	if err != nil {
		return nil, err
	}
	if !info.IsDir() {
		return nil, fmt.Errorf("input directory not found: %s", inputDir)
	}

	options := DefaultBatchParseOptions()
	if opts != nil {
		options = mergeBatchParseOptions(options, *opts)
	}

	cmd, err := p.resolveCLICommand()
	if err != nil {
		return nil, err
	}

	args := append(cmd, "batch-parse", absPath(inputDir), absPath(outputDir))
	args = append(args, buildBatchCLIArgs(options)...)

	_, stderr, exitCode, err := runCommand(args, options.Timeout)
	if err != nil {
		return nil, err
	}
	if exitCode != 0 {
		return nil, &ParseError{
			Message:  "batch parsing failed",
			Stderr:   stderr,
			ExitCode: exitCode,
		}
	}

	return &BatchResult{OutputDir: outputDir}, nil
}

// Screenshot renders one or more page screenshots for a document.
func (p *LiteParse) Screenshot(filePath, outputDir string, opts *ScreenshotOptions) (*ScreenshotBatchResult, error) {
	if _, err := os.Stat(filePath); err != nil {
		return nil, err
	}

	options := DefaultScreenshotOptions()
	if opts != nil {
		options = mergeScreenshotOptions(options, *opts)
	}

	var targetDir string
	if outputDir == "" {
		dir, err := os.MkdirTemp("", "liteparse_screenshots_*")
		if err != nil {
			return nil, err
		}
		targetDir = dir
	} else {
		targetDir = outputDir
		if err := os.MkdirAll(targetDir, 0o755); err != nil {
			return nil, err
		}
	}

	cmd, err := p.resolveCLICommand()
	if err != nil {
		return nil, err
	}

	args := append(cmd,
		"screenshot",
		absPath(filePath),
		"-o", absPath(targetDir),
		"--format", string(options.ImageFormat),
		"--dpi", strconv.Itoa(options.DPI),
		"-q",
	)
	if options.TargetPages != "" {
		args = append(args, "--target-pages", options.TargetPages)
	}

	_, stderr, exitCode, err := runCommand(args, options.Timeout)
	if err != nil {
		return nil, err
	}
	if exitCode != 0 {
		return nil, &ParseError{
			Message:  "screenshot generation failed",
			Stderr:   stderr,
			ExitCode: exitCode,
		}
	}

	screenshots, err := collectScreenshots(targetDir, options.ImageFormat, boolValue(options.LoadBytes, false))
	if err != nil {
		return nil, err
	}

	return &ScreenshotBatchResult{
		Screenshots: screenshots,
		OutputDir:   targetDir,
	}, nil
}

type rawParseResult struct {
	Pages []rawParsedPage `json:"pages"`
}

type rawParsedPage struct {
	Page          int           `json:"page"`
	PageNum       int           `json:"pageNum"`
	Width         float64       `json:"width"`
	Height        float64       `json:"height"`
	Text          string        `json:"text"`
	TextItems     []TextItem    `json:"textItems"`
	BoundingBoxes []BoundingBox `json:"boundingBoxes"`
}

func parseJSONResult(stdout string, outputFormat OutputFormat) (*ParseResult, error) {
	if outputFormat == OutputFormatText {
		return &ParseResult{
			OutputFormat: OutputFormatText,
			Text:         strings.TrimRight(stdout, "\n"),
		}, nil
	}

	var raw rawParseResult
	if err := json.Unmarshal([]byte(stdout), &raw); err != nil {
		trimmed := strings.TrimRight(stdout, "\n")
		if trimmed == "" {
			return nil, err
		}
		return &ParseResult{
			OutputFormat: OutputFormatText,
			Text:         trimmed,
		}, nil
	}

	pages := make([]ParsedPage, 0, len(raw.Pages))
	textParts := make([]string, 0, len(raw.Pages))
	for _, page := range raw.Pages {
		pageNum := page.PageNum
		if pageNum == 0 {
			pageNum = page.Page
		}
		parsed := ParsedPage{
			PageNum:       pageNum,
			Width:         page.Width,
			Height:        page.Height,
			Text:          page.Text,
			TextItems:     page.TextItems,
			BoundingBoxes: page.BoundingBoxes,
		}
		pages = append(pages, parsed)
		textParts = append(textParts, page.Text)
	}

	var rawJSON map[string]any
	if err := json.Unmarshal([]byte(stdout), &rawJSON); err != nil {
		return nil, err
	}

	return &ParseResult{
		OutputFormat: OutputFormatJSON,
		Pages:        pages,
		Text:         strings.Join(textParts, "\n\n"),
		JSON:         rawJSON,
	}, nil
}

func (p *LiteParse) resolveCLICommand() ([]string, error) {
	if strings.TrimSpace(p.cliPath) != "" {
		return strings.Fields(p.cliPath), nil
	}

	if cmd := discoverCLICommand(exec.LookPath, fileExists); len(cmd) > 0 {
		p.cliPath = strings.Join(cmd, " ")
		return cmd, nil
	}

	return nil, &CLINotFoundError{
		Message: "liteparse CLI not found. Install it with: npm i -g @llamaindex/liteparse",
	}
}

func discoverCLICommand(
	lookPath func(string) (string, error),
	exists func(string) bool,
) []string {
	if path, err := lookPath("liteparse"); err == nil {
		return []string{path}
	}

	if path, err := lookPath("lit"); err == nil {
		return []string{path}
	}

	if path, err := lookPath("npx"); err == nil {
		return []string{path, "liteparse"}
	}

	for _, candidate := range cliCandidatePaths() {
		if exists(candidate) {
			return cliCommandFromCandidate(candidate)
		}
	}

	return nil
}

func cliCandidatePaths() []string {
	return []string{
		"./node_modules/.bin/liteparse",
		"../node_modules/.bin/liteparse",
		"../../node_modules/.bin/liteparse",
		"/usr/local/node/bin/liteparse",
		"/usr/local/node/bin/lit",
		"/usr/local/node/bin/npx",
		"/usr/local/bin/liteparse",
		"/usr/local/bin/lit",
		"/usr/local/bin/npx",
		"/opt/homebrew/bin/liteparse",
		"/opt/homebrew/bin/lit",
		"/opt/homebrew/bin/npx",
		"/home/linuxbrew/.linuxbrew/bin/liteparse",
		"/home/linuxbrew/.linuxbrew/bin/lit",
		"/home/linuxbrew/.linuxbrew/bin/npx",
	}
}

func cliCommandFromCandidate(path string) []string {
	if filepath.Base(path) == "npx" {
		return []string{absPath(path), "liteparse"}
	}
	return []string{absPath(path)}
}

func buildParseCLIArgs(opts ParseOptions) []string {
	args := []string{"--format", string(opts.OutputFormat)}
	if !boolValue(opts.OCREnabled, true) {
		args = append(args, "--no-ocr")
	} else if opts.OCRServerURL != "" {
		args = append(args, "--ocr-server-url", opts.OCRServerURL)
	}
	args = append(args, "--ocr-language", opts.OCRLanguage)
	args = append(args, "--max-pages", strconv.Itoa(opts.MaxPages))
	if opts.TargetPages != "" {
		args = append(args, "--target-pages", opts.TargetPages)
	}
	args = append(args, "--dpi", strconv.Itoa(opts.DPI))
	if !boolValue(opts.PreciseBoundingBox, true) {
		args = append(args, "--no-precise-bbox")
	}
	if boolValue(opts.SkipDiagonalText, false) {
		args = append(args, "--skip-diagonal-text")
	}
	if boolValue(opts.PreserveVerySmallText, false) {
		args = append(args, "--preserve-small-text")
	}
	return append(args, "-q")
}

func buildBatchCLIArgs(opts BatchParseOptions) []string {
	args := []string{"--format", string(opts.OutputFormat)}
	if !boolValue(opts.OCREnabled, true) {
		args = append(args, "--no-ocr")
	} else if opts.OCRServerURL != "" {
		args = append(args, "--ocr-server-url", opts.OCRServerURL)
	}
	args = append(args, "--ocr-language", opts.OCRLanguage)
	args = append(args, "--max-pages", strconv.Itoa(opts.MaxPages))
	args = append(args, "--dpi", strconv.Itoa(opts.DPI))
	if !boolValue(opts.PreciseBoundingBox, true) {
		args = append(args, "--no-precise-bbox")
	}
	if boolValue(opts.Recursive, false) {
		args = append(args, "--recursive")
	}
	if opts.ExtensionFilter != "" {
		args = append(args, "--extension", opts.ExtensionFilter)
	}
	return args
}

func mergeParseOptions(base ParseOptions, override ParseOptions) ParseOptions {
	if override.OutputFormat != "" {
		base.OutputFormat = override.OutputFormat
	}
	if override.OCREnabled != nil {
		base.OCREnabled = override.OCREnabled
	}
	if override.OCRServerURL != "" {
		base.OCRServerURL = override.OCRServerURL
	}
	if override.OCRLanguage != "" {
		base.OCRLanguage = override.OCRLanguage
	}
	if override.MaxPages != 0 {
		base.MaxPages = override.MaxPages
	}
	if override.TargetPages != "" {
		base.TargetPages = override.TargetPages
	}
	if override.DPI != 0 {
		base.DPI = override.DPI
	}
	if override.PreciseBoundingBox != nil {
		base.PreciseBoundingBox = override.PreciseBoundingBox
	}
	if override.SkipDiagonalText != nil {
		base.SkipDiagonalText = override.SkipDiagonalText
	}
	if override.PreserveVerySmallText != nil {
		base.PreserveVerySmallText = override.PreserveVerySmallText
	}
	if override.Timeout > 0 {
		base.Timeout = override.Timeout
	}
	return base
}

func mergeBatchParseOptions(base BatchParseOptions, override BatchParseOptions) BatchParseOptions {
	if override.OutputFormat != "" {
		base.OutputFormat = override.OutputFormat
	}
	if override.OCREnabled != nil {
		base.OCREnabled = override.OCREnabled
	}
	if override.OCRServerURL != "" {
		base.OCRServerURL = override.OCRServerURL
	}
	if override.OCRLanguage != "" {
		base.OCRLanguage = override.OCRLanguage
	}
	if override.MaxPages != 0 {
		base.MaxPages = override.MaxPages
	}
	if override.DPI != 0 {
		base.DPI = override.DPI
	}
	if override.PreciseBoundingBox != nil {
		base.PreciseBoundingBox = override.PreciseBoundingBox
	}
	if override.Recursive != nil {
		base.Recursive = override.Recursive
	}
	if override.ExtensionFilter != "" {
		base.ExtensionFilter = override.ExtensionFilter
	}
	if override.Timeout > 0 {
		base.Timeout = override.Timeout
	}
	return base
}

func mergeScreenshotOptions(base ScreenshotOptions, override ScreenshotOptions) ScreenshotOptions {
	if override.TargetPages != "" {
		base.TargetPages = override.TargetPages
	}
	if override.DPI != 0 {
		base.DPI = override.DPI
	}
	if override.ImageFormat != "" {
		base.ImageFormat = override.ImageFormat
	}
	if override.LoadBytes != nil {
		base.LoadBytes = override.LoadBytes
	}
	if override.Timeout > 0 {
		base.Timeout = override.Timeout
	}
	return base
}

func runCommand(args []string, timeout time.Duration) (stdout string, stderr string, exitCode int, err error) {
	ctx := context.Background()
	if timeout > 0 {
		var cancel context.CancelFunc
		ctx, cancel = context.WithTimeout(ctx, timeout)
		defer cancel()
	}

	cmd := exec.CommandContext(ctx, args[0], args[1:]...)
	var stdoutBuf bytes.Buffer
	var stderrBuf bytes.Buffer
	cmd.Stdout = &stdoutBuf
	cmd.Stderr = &stderrBuf

	runErr := cmd.Run()
	stdout = stdoutBuf.String()
	stderr = stderrBuf.String()

	if runErr == nil {
		return stdout, stderr, 0, nil
	}

	// Check for timeout error
	if errors.Is(runErr, context.DeadlineExceeded) {
		return stdout, stderr, 0, fmt.Errorf("operation timed out after %s", timeout)
	}

	// Check for exit error (non-zero exit code)
	var exitErr *exec.ExitError
	if errors.As(runErr, &exitErr) {
		return stdout, stderr, exitErr.ExitCode(), nil
	}

	// Other errors (e.g., command not found)
	return stdout, stderr, 0, runErr
}

func collectScreenshots(outputDir string, imageFormat ImageFormat, loadBytes bool) ([]ScreenshotResult, error) {
	pattern := filepath.Join(outputDir, "*."+string(imageFormat))
	matches, err := filepath.Glob(pattern)
	if err != nil {
		return nil, err
	}
	sort.Strings(matches)

	results := make([]ScreenshotResult, 0, len(matches))
	for _, path := range matches {
		base := strings.TrimSuffix(filepath.Base(path), filepath.Ext(path))
		if !strings.HasPrefix(base, "page_") {
			continue
		}
		pageNum, err := strconv.Atoi(strings.TrimPrefix(base, "page_"))
		if err != nil {
			continue
		}

		item := ScreenshotResult{
			PageNum:   pageNum,
			ImagePath: path,
		}
		if loadBytes {
			data, err := os.ReadFile(path)
			if err != nil {
				return nil, err
			}
			item.ImageBytes = data
		}
		results = append(results, item)
	}
	return results, nil
}

func absPath(path string) string {
	absolute, err := filepath.Abs(path)
	if err != nil {
		return path
	}
	return absolute
}

func boolValue(v *bool, fallback bool) bool {
	if v == nil {
		return fallback
	}
	return *v
}

func fileExists(path string) bool {
	info, err := os.Stat(path)
	return err == nil && !info.IsDir()
}
