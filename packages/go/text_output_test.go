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

// TestPrintTextOutput 解析 PDF 并直接打印文本内容
func TestPrintTextOutput(t *testing.T) {
	pdfPath := "/Users/xxdld/Documents/DataDictionary_20260122153143.pdf"

	if _, err := os.Stat(pdfPath); os.IsNotExist(err) {
		t.Skipf("PDF file not found: %s", pdfPath)
	}

	parser := New("")

	opts := DefaultParseOptions()
	opts.OutputFormat = OutputFormatText
	opts.OCREnabled = Bool(false)
	opts.Timeout = 30 * time.Second

	result, err := parser.Parse(pdfPath, &opts)
	if err != nil {
		t.Fatalf("Parse failed: %v", err)
	}

	t.Logf("\n========== PDF 文本内容 ==========\n")
	t.Logf("文件: %s", pdfPath)
	t.Logf("文本长度: %d 字符\n", len(result.Text))
	t.Logf("========== 开始 ==========\n\n%s\n\n========== 结束 ==========", result.Text)
}
