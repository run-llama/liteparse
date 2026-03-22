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
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// TestParseAndSaveResults 解析 PDF 并保存详细结果到文件
func TestParseAndSaveResults(t *testing.T) {
	pdfPath := "/Users/xxdld/Documents/DataDictionary_20260122153143.pdf"

	if _, err := os.Stat(pdfPath); os.IsNotExist(err) {
		t.Skipf("PDF file not found: %s", pdfPath)
	}

	parser := New("")
	outputDir := t.TempDir()

	// 测试 1: JSON 格式
	t.Run("JSON Format", func(t *testing.T) {
		opts := DefaultParseOptions()
		opts.OutputFormat = OutputFormatJSON
		opts.OCREnabled = Bool(false)
		opts.Timeout = 30 * time.Second

		result, err := parser.Parse(pdfPath, &opts)
		if err != nil {
			t.Fatalf("Parse failed: %v", err)
		}

		// 保存完整的 JSON 结果
		jsonFile := filepath.Join(outputDir, "result.json")
		jsonData, _ := json.MarshalIndent(result.JSON, "", "  ")
		if err := os.WriteFile(jsonFile, jsonData, 0644); err != nil {
			t.Logf("Warning: failed to write JSON file: %v", err)
		} else {
			t.Logf("JSON result saved to: %s", jsonFile)
		}

		// 保存提取的文本
		textFile := filepath.Join(outputDir, "extracted_text.txt")
		if err := os.WriteFile(textFile, []byte(result.Text), 0644); err != nil {
			t.Logf("Warning: failed to write text file: %v", err)
		} else {
			t.Logf("Extracted text saved to: %s", textFile)
		}

		t.Logf("JSON Format Results:")
		t.Logf("  - Pages: %d", result.NumPages())
		t.Logf("  - Text length: %d", len(result.Text))
		t.Logf("  - First page text items: %d", len(result.GetPage(1).TextItems))
	})

	// 测试 2: 纯文本格式
	t.Run("Text Format", func(t *testing.T) {
		opts := DefaultParseOptions()
		opts.OutputFormat = OutputFormatText
		opts.OCREnabled = Bool(false)
		opts.Timeout = 30 * time.Second

		result, err := parser.Parse(pdfPath, &opts)
		if err != nil {
			t.Fatalf("Parse failed: %v", err)
		}

		// 保存纯文本结果
		textFile := filepath.Join(outputDir, "plain_text.txt")
		if err := os.WriteFile(textFile, []byte(result.Text), 0644); err != nil {
			t.Logf("Warning: failed to write text file: %v", err)
		} else {
			t.Logf("Plain text saved to: %s", textFile)
		}

		t.Logf("Text Format Results:")
		t.Logf("  - Text length: %d", len(result.Text))
		t.Logf("  - Has structured pages: %v", result.NumPages() > 0)
	})

	t.Logf("\nAll output files saved to: %s", outputDir)
}
