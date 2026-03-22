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
	"os"
	"testing"
	"time"
)

// TestOCRComparison 对比启用和不启用 OCR 的解析结果
func TestOCRComparison(t *testing.T) {
	pdfPath := "/Users/xxdld/Documents/DataDictionary_20260122153143.pdf"

	if _, err := os.Stat(pdfPath); os.IsNotExist(err) {
		t.Skipf("PDF file not found: %s", pdfPath)
	}

	parser := New("")

	// 测试 1: 不启用 OCR
	t.Run("Without OCR", func(t *testing.T) {
		opts := DefaultParseOptions()
		opts.OutputFormat = OutputFormatJSON
		opts.OCREnabled = Bool(false)
		opts.Timeout = 30 * time.Second

		start := time.Now()
		result, err := parser.Parse(pdfPath, &opts)
		duration := time.Since(start)

		if err != nil {
			t.Fatalf("Parse without OCR failed: %v", err)
		}

		t.Logf("Without OCR Results:")
		t.Logf("  - Duration: %v", duration)
		t.Logf("  - Pages: %d", result.NumPages())
		t.Logf("  - Text length: %d characters", len(result.Text))
		t.Logf("  - First page text items: %d", len(result.GetPage(1).TextItems))

		// 显示第一页的前 200 个字符
		if len(result.Text) > 0 {
			preview := result.Text
			if len(preview) > 200 {
				preview = preview[:200] + "..."
			}
			t.Logf("  - Text preview: %s", preview)
		}
	})

	// 测试 2: 启用 OCR
	t.Run("With OCR", func(t *testing.T) {
		opts := DefaultParseOptions()
		opts.OutputFormat = OutputFormatJSON
		opts.OCREnabled = Bool(true)
		opts.Timeout = 60 * time.Second

		start := time.Now()
		result, err := parser.Parse(pdfPath, &opts)
		duration := time.Since(start)

		if err != nil {
			t.Fatalf("Parse with OCR failed: %v", err)
		}

		t.Logf("With OCR Results:")
		t.Logf("  - Duration: %v", duration)
		t.Logf("  - Pages: %d", result.NumPages())
		t.Logf("  - Text length: %d characters", len(result.Text))
		t.Logf("  - First page text items: %d", len(result.GetPage(1).TextItems))

		// 显示第一页的前 200 个字符
		if len(result.Text) > 0 {
			preview := result.Text
			if len(preview) > 200 {
				preview = preview[:200] + "..."
			}
			t.Logf("  - Text preview: %s", preview)
		}
	})

	// 测试 3: OCR + 纯文本格式
	t.Run("With OCR - Text Format", func(t *testing.T) {
		opts := DefaultParseOptions()
		opts.OutputFormat = OutputFormatText
		opts.OCREnabled = Bool(true)
		opts.Timeout = 60 * time.Second

		start := time.Now()
		result, err := parser.Parse(pdfPath, &opts)
		duration := time.Since(start)

		if err != nil {
			t.Fatalf("Parse with OCR (text format) failed: %v", err)
		}

		t.Logf("With OCR (Text Format) Results:")
		t.Logf("  - Duration: %v", duration)
		t.Logf("  - Text length: %d characters", len(result.Text))
		t.Logf("  - Output format: %s", result.OutputFormat)
	})
}

// TestOCRPerformance 测试 OCR 性能和准确性
func TestOCRPerformance(t *testing.T) {
	pdfPath := "/Users/xxdld/Documents/DataDictionary_20260122153143.pdf"

	if _, err := os.Stat(pdfPath); os.IsNotExist(err) {
		t.Skipf("PDF file not found: %s", pdfPath)
	}

	parser := New("")

	// 只解析前 3 页来测试性能
	opts := DefaultParseOptions()
	opts.OutputFormat = OutputFormatJSON
	opts.OCREnabled = Bool(true)
	opts.TargetPages = "1-3"
	opts.Timeout = 30 * time.Second

	start := time.Now()
	result, err := parser.Parse(pdfPath, &opts)
	duration := time.Since(start)

	if err != nil {
		t.Fatalf("Parse failed: %v", err)
	}

	t.Logf("OCR Performance Test (Pages 1-3):")
	t.Logf("  - Total duration: %v", duration)
	t.Logf("  - Average per page: %v", duration/3)
	t.Logf("  - Pages parsed: %d", result.NumPages())
	t.Logf("  - Total text length: %d characters", len(result.Text))

	// 显示每一页的详细信息
	for i := 1; i <= result.NumPages(); i++ {
		page := result.GetPage(i)
		if page != nil {
			t.Logf("  - Page %d: %d text items, %d characters",
				i, len(page.TextItems), len(page.Text))
		}
	}
}
