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
	"encoding/base64"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"
)

func TestParse(t *testing.T) {
	input := writeTempFile(t, "document.pdf", "dummy")

	opts := DefaultParseOptions()
	opts.OCREnabled = Bool(false)
	opts.TargetPages = "1-2"
	opts.PreserveVerySmallText = Bool(true)
	opts.SkipDiagonalText = Bool(true)
	opts.Timeout = 2 * time.Second

	parser := New(helperCLIPath(t))
	result, err := parser.Parse(input, &opts)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	if result.NumPages() != 2 {
		t.Fatalf("expected 2 pages, got %d", result.NumPages())
	}
	if result.OutputFormat != OutputFormatJSON {
		t.Fatalf("expected json output format, got %q", result.OutputFormat)
	}
	if result.Text != "hello page 1\n\nhello page 2" {
		t.Fatalf("unexpected text: %q", result.Text)
	}
	if page := result.GetPage(2); page == nil || page.Text != "hello page 2" {
		t.Fatalf("expected page 2 text, got %#v", page)
	}
}

func TestParseTextOutput(t *testing.T) {
	input := writeTempFile(t, "document.pdf", "dummy")

	opts := DefaultParseOptions()
	opts.OutputFormat = OutputFormatText
	opts.TargetPages = "3-4"
	opts.Timeout = 2 * time.Second

	parser := New(helperCLIPath(t))
	result, err := parser.Parse(input, &opts)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	if result.Text != "plain text output" {
		t.Fatalf("unexpected text: %q", result.Text)
	}
	if result.OutputFormat != OutputFormatText {
		t.Fatalf("expected text output format, got %q", result.OutputFormat)
	}
	if result.NumPages() != 0 {
		t.Fatalf("expected no structured pages for text output, got %d", result.NumPages())
	}
	if result.JSON != nil {
		t.Fatalf("expected nil JSON payload for text output")
	}
}

func TestBatchParse(t *testing.T) {
	inputDir := t.TempDir()
	outputDir := filepath.Join(t.TempDir(), "out")
	writeTempFileAt(t, inputDir, "a.pdf", "dummy")

	opts := DefaultBatchParseOptions()
	opts.OutputFormat = OutputFormatJSON
	opts.Recursive = Bool(true)
	opts.ExtensionFilter = ".pdf"
	opts.Timeout = 2 * time.Second

	parser := New(helperCLIPath(t))
	result, err := parser.BatchParse(inputDir, outputDir, &opts)
	if err != nil {
		t.Fatalf("BatchParse returned error: %v", err)
	}

	if result.OutputDir != outputDir {
		t.Fatalf("unexpected output dir: %q", result.OutputDir)
	}
	if _, err := os.Stat(filepath.Join(outputDir, "done.txt")); err != nil {
		t.Fatalf("expected fake batch output file: %v", err)
	}
}

func TestBatchParseTextFormat(t *testing.T) {
	inputDir := t.TempDir()
	outputDir := filepath.Join(t.TempDir(), "out")
	writeTempFileAt(t, inputDir, "a.pdf", "dummy")

	opts := DefaultBatchParseOptions()
	opts.OutputFormat = OutputFormatText
	opts.Timeout = 2 * time.Second

	parser := New(helperCLIPath(t))
	if _, err := parser.BatchParse(inputDir, outputDir, &opts); err != nil {
		t.Fatalf("BatchParse returned error: %v", err)
	}
}

func TestScreenshot(t *testing.T) {
	input := writeTempFile(t, "document.pdf", "dummy")
	outputDir := filepath.Join(t.TempDir(), "shots")

	opts := DefaultScreenshotOptions()
	opts.TargetPages = "1-2"
	opts.LoadBytes = Bool(true)
	opts.Timeout = 2 * time.Second

	parser := New(helperCLIPath(t))
	result, err := parser.Screenshot(input, outputDir, &opts)
	if err != nil {
		t.Fatalf("Screenshot returned error: %v", err)
	}

	if result.Len() != 2 {
		t.Fatalf("expected 2 screenshots, got %d", result.Len())
	}
	if shot := result.GetPage(1); shot == nil || len(shot.ImageBytes) == 0 {
		t.Fatalf("expected screenshot bytes for page 1, got %#v", shot)
	}
}

func TestParseError(t *testing.T) {
	input := writeTempFile(t, "document.pdf", "dummy")
	parser := New(helperCLIPath(t, "--fail-parse"))

	_, err := parser.Parse(input, nil)
	if err == nil {
		t.Fatal("expected parse error")
	}

	parseErr, ok := err.(*ParseError)
	if !ok {
		t.Fatalf("expected *ParseError, got %T", err)
	}
	if parseErr.ExitCode != 9 {
		t.Fatalf("expected exit code 9, got %d", parseErr.ExitCode)
	}
	if !strings.Contains(parseErr.Stderr, "forced parse failure") {
		t.Fatalf("unexpected stderr: %q", parseErr.Stderr)
	}
}

func TestCLINotFound(t *testing.T) {
	lookPath := func(string) (string, error) {
		return "", fmt.Errorf("not found")
	}
	exists := func(string) bool {
		return false
	}

	cmd := discoverCLICommand(lookPath, exists)
	if cmd != nil {
		t.Fatalf("expected no command, got %#v", cmd)
	}
}

func TestDiscoverCLICommandFallsBackToUsrLocalNodeBin(t *testing.T) {
	lookPath := func(string) (string, error) {
		return "", fmt.Errorf("not found")
	}
	exists := func(path string) bool {
		return path == "/usr/local/node/bin/liteparse"
	}

	cmd := discoverCLICommand(lookPath, exists)
	if len(cmd) != 1 || cmd[0] != "/usr/local/node/bin/liteparse" {
		t.Fatalf("unexpected command: %#v", cmd)
	}
}

func TestDiscoverCLICommandFallsBackToUsrLocalNodeBinNpx(t *testing.T) {
	lookPath := func(string) (string, error) {
		return "", fmt.Errorf("not found")
	}
	exists := func(path string) bool {
		return path == "/usr/local/node/bin/npx"
	}

	cmd := discoverCLICommand(lookPath, exists)
	if len(cmd) != 2 || cmd[0] != "/usr/local/node/bin/npx" || cmd[1] != "liteparse" {
		t.Fatalf("unexpected command: %#v", cmd)
	}
}

func TestHelperProcess(t *testing.T) {
	if os.Getenv("GO_WANT_HELPER_PROCESS") != "1" {
		return
	}

	args := helperArgs(os.Args)
	failParse := false
	if len(args) > 0 && args[0] == "--fail-parse" {
		failParse = true
		args = args[1:]
	}
	if len(args) == 0 {
		fmt.Fprintln(os.Stderr, "missing helper command")
		os.Exit(2)
	}

	switch args[0] {
	case "parse":
		if failParse {
			fmt.Fprintln(os.Stderr, "forced parse failure")
			os.Exit(9)
		}
		requireFlag(args, "-q")
		switch mustArgValue(args, "--format") {
		case "json":
			requireFlag(args, "--no-ocr", "--preserve-small-text", "--skip-diagonal-text")
			requireOptionValue(args, "--target-pages", "1-2")
			fmt.Fprint(os.Stdout, `{"pages":[{"page":1,"width":612,"height":792,"text":"hello page 1","textItems":[{"str":"hello","x":1,"y":2,"width":3,"height":4,"w":3,"h":4,"r":0}],"boundingBoxes":[{"x1":1,"y1":2,"x2":3,"y2":4}]},{"pageNum":2,"width":612,"height":792,"text":"hello page 2","textItems":[],"boundingBoxes":[]}]}`)
		case "text":
			requireOptionValue(args, "--target-pages", "3-4")
			fmt.Fprint(os.Stdout, "plain text output\n")
		default:
			fmt.Fprintf(os.Stderr, "unsupported parse format: %s\n", mustArgValue(args, "--format"))
			os.Exit(2)
		}
	case "batch-parse":
		switch mustArgValue(args, "--format") {
		case "json":
			requireFlag(args, "--recursive")
			requireOptionValue(args, "--extension", ".pdf")
		case "text":
		default:
			fmt.Fprintf(os.Stderr, "unsupported batch format: %s\n", mustArgValue(args, "--format"))
			os.Exit(2)
		}
		if len(args) < 3 {
			fmt.Fprintln(os.Stderr, "missing directories")
			os.Exit(2)
		}
		outputDir := args[2]
		if err := os.MkdirAll(outputDir, 0o755); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(2)
		}
		if err := os.WriteFile(filepath.Join(outputDir, "done.txt"), []byte("ok"), 0o644); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(2)
		}
	case "screenshot":
		requireFlag(args, "-q")
		requireOptionValue(args, "--format", "png")
		requireOptionValue(args, "--dpi", "150")
		requireOptionValue(args, "--target-pages", "1-2")
		outputDir, ok := argValue(args, "-o")
		if !ok {
			fmt.Fprintln(os.Stderr, "missing output dir")
			os.Exit(2)
		}
		if err := os.MkdirAll(outputDir, 0o755); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(2)
		}
		data, _ := base64.StdEncoding.DecodeString("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a5s8AAAAASUVORK5CYII=")
		for _, name := range []string{"page_1.png", "page_2.png"} {
			if err := os.WriteFile(filepath.Join(outputDir, name), data, 0o644); err != nil {
				fmt.Fprintln(os.Stderr, err)
				os.Exit(2)
			}
		}
	default:
		fmt.Fprintf(os.Stderr, "unsupported helper command: %s\n", args[0])
		os.Exit(2)
	}

	os.Exit(0)
}

func helperCLIPath(t *testing.T, extraArgs ...string) string {
	t.Helper()
	if runtime.GOOS == "windows" {
		t.Skip("helper CLI wrapper uses /bin/sh")
	}

	scriptPath := filepath.Join(t.TempDir(), "fake-liteparse")
	forwarded := ""
	if len(extraArgs) > 0 {
		forwarded = " " + strings.Join(extraArgs, " ")
	}
	script := fmt.Sprintf("#!/bin/sh\nGO_WANT_HELPER_PROCESS=1 exec %q -test.run=TestHelperProcess --%s \"$@\"\n", os.Args[0], forwarded)
	if err := os.WriteFile(scriptPath, []byte(script), 0o755); err != nil {
		t.Fatalf("write helper CLI script: %v", err)
	}
	return scriptPath
}

func helperArgs(args []string) []string {
	for i, arg := range args {
		if arg == "--" {
			return args[i+1:]
		}
	}
	return nil
}

func requireFlag(args []string, flags ...string) {
	for _, flag := range flags {
		if !contains(args, flag) {
			fmt.Fprintf(os.Stderr, "missing flag %s\n", flag)
			os.Exit(2)
		}
	}
}

func requireOptionValue(args []string, key, expected string) {
	if mustArgValue(args, key) != expected {
		fmt.Fprintf(os.Stderr, "missing or invalid %s\n", key)
		os.Exit(2)
	}
}

func mustArgValue(args []string, key string) string {
	actual, ok := argValue(args, key)
	if !ok {
		fmt.Fprintf(os.Stderr, "missing option %s\n", key)
		os.Exit(2)
	}
	return actual
}

func argValue(args []string, key string) (string, bool) {
	for i := 0; i < len(args)-1; i++ {
		if args[i] == key {
			return args[i+1], true
		}
	}
	return "", false
}

func contains(args []string, target string) bool {
	for _, arg := range args {
		if arg == target {
			return true
		}
	}
	return false
}

func writeTempFile(t *testing.T, name, content string) string {
	t.Helper()
	return writeTempFileAt(t, t.TempDir(), name, content)
}

func writeTempFileAt(t *testing.T, dir, name, content string) string {
	t.Helper()
	path := filepath.Join(dir, name)
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatalf("write temp file: %v", err)
	}
	return path
}

func TestParseRealPDF(t *testing.T) {
	// 测试解析真实的 PDF 文件
	pdfPath := "/Users/xxdld/Documents/DataDictionary_20260122153143.pdf"

	// 检查文件是否存在
	if _, err := os.Stat(pdfPath); os.IsNotExist(err) {
		t.Skipf("PDF file not found: %s", pdfPath)
	}

	// 创建解析器实例（使用系统中的 liteparse CLI）
	parser := New("")

	// 配置解析选项
	opts := DefaultParseOptions()
	opts.OCREnabled = Bool(false)           // 禁用 OCR
	opts.OutputFormat = OutputFormatJSON    // 输出 JSON 格式
	opts.PreserveVerySmallText = Bool(true) // 保留小文本
	opts.Timeout = 30 * time.Second         // 设置 30 秒超时

	// 执行解析
	result, err := parser.Parse(pdfPath, &opts)
	if err != nil {
		t.Fatalf("Parse failed: %v", err)
	}

	// 验证结果
	if result == nil {
		t.Fatal("Expected non-nil result")
	}

	t.Logf("Successfully parsed PDF:")
	t.Logf("  - Number of pages: %d", result.NumPages())
	t.Logf("  - Output format: %s", result.OutputFormat)
	t.Logf("  - Text length: %d characters", len(result.Text))

	// 打印前 500 个字符的文本内容
	if len(result.Text) > 0 {
		preview := result.Text
		if len(preview) > 500 {
			preview = preview[:500] + "..."
		}
		t.Logf("  - Text preview:\n%s", preview)
	}

	// 验证基本结构
	if result.NumPages() == 0 {
		t.Error("Expected at least one page")
	}

	if result.OutputFormat != OutputFormatJSON {
		t.Errorf("Expected JSON format, got %s", result.OutputFormat)
	}

	// 打印每一页的信息
	for i := 1; i <= result.NumPages(); i++ {
		page := result.GetPage(i)
		if page != nil {
			t.Logf("  - Page %d: %d text items, text length: %d",
				i, len(page.TextItems), len(page.Text))
		}
	}
}

func TestParseRealPDFWithOCR(t *testing.T) {
	// 测试使用 OCR 解析 PDF
	pdfPath := "/Users/xxdld/Documents/DataDictionary_20260122153143.pdf"

	if _, err := os.Stat(pdfPath); os.IsNotExist(err) {
		t.Skipf("PDF file not found: %s", pdfPath)
	}

	parser := New("")

	opts := DefaultParseOptions()
	opts.OCREnabled = Bool(true)         // 启用 OCR
	opts.OutputFormat = OutputFormatJSON
	opts.Timeout = 60 * time.Second      // OCR 需要更长时间

	result, err := parser.Parse(pdfPath, &opts)
	if err != nil {
		t.Fatalf("Parse with OCR failed: %v", err)
	}

	if result == nil {
		t.Fatal("Expected non-nil result")
	}

	t.Logf("Successfully parsed PDF with OCR:")
	t.Logf("  - Number of pages: %d", result.NumPages())
	t.Logf("  - Text length: %d characters", len(result.Text))
}

func TestParseRealPDFTextFormat(t *testing.T) {
	// 测试以纯文本格式解析 PDF
	pdfPath := "/Users/xxdld/Documents/DataDictionary_20260122153143.pdf"

	if _, err := os.Stat(pdfPath); os.IsNotExist(err) {
		t.Skipf("PDF file not found: %s", pdfPath)
	}

	parser := New("")

	opts := DefaultParseOptions()
	opts.OutputFormat = OutputFormatText  // 纯文本输出
	opts.OCREnabled = Bool(false)
	opts.Timeout = 30 * time.Second

	result, err := parser.Parse(pdfPath, &opts)
	if err != nil {
		t.Fatalf("Parse as text failed: %v", err)
	}

	if result == nil {
		t.Fatal("Expected non-nil result")
	}

	t.Logf("Successfully parsed PDF as text:")
	t.Logf("  - Output format: %s", result.OutputFormat)
	t.Logf("  - Text length: %d characters", len(result.Text))

	if result.OutputFormat != OutputFormatText {
		t.Errorf("Expected text format, got %s", result.OutputFormat)
	}

	// 纯文本格式不应该有结构化的页面数据
	if result.NumPages() != 0 {
		t.Logf("Warning: Text format should not have structured pages, got %d", result.NumPages())
	}

	// 打印文本内容
	if len(result.Text) > 0 {
		preview := result.Text
		if len(preview) > 1000 {
			preview = preview[:1000] + "..."
		}
		t.Logf("  - Full text:\n%s", preview)
	}
}
